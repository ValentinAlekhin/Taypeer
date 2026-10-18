"""Guard against losing saved/live edits and losing old previews on export failure."""
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import design
import verify


class SharedPalette(unittest.TestCase):
    def palette_file(self, directory, change=None):
        modes = {mode: {role: 0x123456 for role in design.DESIGN_ROLES.values()}
                 for mode in ['light', 'dark']}
        if change:
            change(modes)
        path = Path(directory)/'theme.toml'
        path.write_text('\n'.join(f'[{mode}]\n' + '\n'.join(
            f'{role} = {value}' for role, value in roles.items())
            for mode, roles in modes.items()))
        return path

    def test_product_roles_map_to_fig_names(self):
        with tempfile.TemporaryDirectory() as directory:
            palettes = design.design_palettes(self.palette_file(directory))
        self.assertEqual(palettes['dark']['bg'], '#123456')
        self.assertEqual(palettes['light']['onPrimary'], '#123456')
        self.assertEqual(set(palettes['dark']), set(design.DESIGN_ROLES))

    def test_incomplete_or_invalid_palette_fails_before_build(self):
        for change in [lambda modes: modes['dark'].pop('raised'),
                       lambda modes: modes['light'].update(background=0x1000000)]:
            with self.subTest(change=change), tempfile.TemporaryDirectory() as directory:
                with self.assertRaises(ValueError):
                    design.design_palettes(self.palette_file(directory, change))

    def test_palette_change_invalidates_candidate_freshness(self):
        with tempfile.TemporaryDirectory() as directory:
            palette = self.palette_file(directory)
            candidate, manifest = Path(directory)/'candidate.fig', Path(directory)/'candidate.json'
            candidate.write_bytes(b'generated')
            with patch.object(design, 'PALETTE', palette), patch.object(design, 'CANDIDATE', candidate), patch.object(design, 'MANIFEST', manifest):
                manifest.write_text(json.dumps({'inputs_sha256':design.input_hash(),
                                                'candidate_sha256':design.file_hash(candidate)}))
                design.check_candidate()
                palette.write_text(palette.read_text().replace('1193046', '1193047', 1))
                with self.assertRaises(RuntimeError):
                    design.check_candidate()


class PublishSafety(unittest.TestCase):
    def setUp(self):
        self.state = dict(current_file='saved', expected_file='saved', current_scene='old',
                          live_scene='old', candidate_scene='new', baseline_scene='old')

    def test_normal_source_edit_can_publish(self):
        design.assert_publish_safe(**self.state)

    def test_unsaved_app_edit_blocks_even_if_saved_file_is_unchanged(self):
        self.state['live_scene'] = 'manual'
        with self.assertRaises(RuntimeError):
            design.assert_publish_safe(**self.state)

    def test_saved_edit_after_build_blocks(self):
        self.state['current_file'] = 'saved-after-build'
        with self.assertRaises(RuntimeError):
            design.assert_publish_safe(**self.state)

    def test_saved_edit_before_build_is_not_lost(self):
        self.state.update(current_scene='manual', live_scene='manual')
        with self.assertRaises(RuntimeError):
            design.assert_publish_safe(**self.state)
        # Porting the entire manual scene into source allows adoption.
        self.state['candidate_scene'] = 'manual'
        design.assert_publish_safe(**self.state)

    def test_initial_baseline_requires_reproduction(self):
        self.state['baseline_scene'] = None
        with self.assertRaises(RuntimeError):
            design.assert_publish_safe(**self.state)
        self.state['candidate_scene'] = 'old'
        design.assert_publish_safe(**self.state)

    def test_candidate_mutation_requires_rebuild(self):
        with tempfile.TemporaryDirectory() as directory:
            candidate, manifest = Path(directory)/'candidate.fig', Path(directory)/'candidate.json'
            candidate.write_bytes(b'generated')
            manifest.write_text(json.dumps({'inputs_sha256':'inputs', 'candidate_sha256':design.file_hash(candidate)}))
            with patch.object(design, 'CANDIDATE', candidate), patch.object(design, 'MANIFEST', manifest), patch.object(design, 'input_hash', return_value='inputs'):
                design.check_candidate()
                candidate.write_bytes(b'manual candidate edit')
                with self.assertRaises(RuntimeError):
                    design.check_candidate()

    def test_live_backup_is_parsed_without_dropping_vectors_or_manual_edits(self):
        manual = [{'name':'Manual edit', 'children':[{'type':'INSTANCE', 'children':[
            {'type':'VECTOR', 'vectorPaths':[{'data':'M0 0L20 20'}]}]}]}]
        saved = []

        def save(document_id, path):
            self.assertEqual(document_id, 'correct-tab')
            path.write_bytes(b'complete live fig')
            saved.append(path)

        def cli(*args, **kwargs):
            self.assertEqual(args[0], 'eval')
            self.assertEqual(Path(args[1]).read_bytes(), b'complete live fig')
            return json.dumps(manual)

        with tempfile.TemporaryDirectory() as directory:
            with patch.object(design, 'BUILD', Path(directory)), patch.object(design, 'save_live_copy', side_effect=save), patch.object(design, 'run_cli', side_effect=cli):
                actual = design.scene(document_id='correct-tab')
            self.assertEqual(actual, manual)
            self.assertTrue(saved[0].is_file())
        self.state['live_scene'] = design.json_hash(actual)
        with self.assertRaises(RuntimeError):
            design.assert_publish_safe(**self.state)



class ExportSafety(unittest.TestCase):
    @unittest.skipUnless(shutil.which('node'), 'Node is needed to evaluate the OpenPencil index query')
    def test_page_headings_are_not_indexed_or_exported(self):
        from PIL import Image
        scene = {'root': {'children': [{'name': '01 · Desktop', 'children': [
            {'id': 'heading', 'type': 'TEXT', 'name': 'Рабочие экраны', 'width': 120, 'height': 24},
            {'id': 'screen', 'type': 'FRAME', 'name': 'macOS / main', 'width': 80, 'height': 60},
        ]}]}}
        exported = []

        def cli(*args, **kwargs):
            if args[0] == 'eval':
                code = 'const figma=' + json.dumps(scene) + ';' + \
                       'console.log(JSON.stringify(new Function("figma",' + json.dumps(args[3]) + ')(figma)));'
                return subprocess.run(['node', '-e', code], check=True, capture_output=True, text=True).stdout
            exported.append(args[3])
            Image.new('RGB', (80, 60)).save(Path(args[-1]))
            return ''

        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            fig = out/'input.fig'
            fig.write_bytes(b'fixture')
            result = verify.export(fig, out, cli)
            self.assertEqual(result['exported'], 1)
            self.assertEqual(exported, ['screen'])
            self.assertEqual(json.loads((out/'screen-index.json').read_text()), [
                {'id': 'screen', 'name': 'macOS / main', 'page': '01 · Desktop', 'w': 80, 'h': 60}])
            self.assertFalse((out/'previews'/'рабочие-экраны.png').exists())

    def test_android_sheets_do_not_mix_with_phone_contact_sheets(self):
        from PIL import Image
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            fig = out/'input.fig'
            fig.write_bytes(b'fixture')
            nodes = [{'id':str(i), 'name':name, 'page':'Android', 'w':100, 'h':100}
                     for i, name in enumerate(['Android / entries', 'Android form / create',
                                               'Android states / system', 'Android QA / small'])]

            def cli(*args, **kwargs):
                if args[0] == 'eval':
                    return json.dumps(nodes)
                Image.new('RGB', (100, 100)).save(Path(args[-1]))
                return ''

            verify.export(fig, out, cli)
            for group in ['android', 'android-form', 'android-states', 'android-qa']:
                self.assertTrue((out/'previews'/f'{group}-contact.png').is_file())
            # One row in each sheet: a broad Android prefix must not collect all.
            with Image.open(out/'previews'/'android-contact.png') as sheet:
                self.assertEqual(sheet.height, 484)

    def test_large_sets_export_every_frame_and_paginate_contact_sheets(self):
        from PIL import Image
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            fig = out/'input.fig'
            fig.write_bytes(b'fixture')
            nodes = [{'id':f'1:{i}', 'name':f'Dialog / example-{i}',
                      'page':'Dialogs', 'w':80, 'h':60} for i in range(13)]

            def cli(*args, **kwargs):
                if args[0] == 'eval':
                    return json.dumps(nodes)
                Image.new('RGB', (80, 60), '#111111').save(Path(args[-1]))
                return ''

            result = verify.export(fig, out, cli)
            self.assertEqual(result['exported'], 13)
            previews = out/'previews'
            self.assertTrue((previews/'dialog-contact.png').is_file())
            self.assertTrue((previews/'dialog-contact-02.png').is_file())
            self.assertEqual(len(list(previews.glob('dialog-example-*.png'))), 13)
            self.assertEqual(len(json.loads((previews/'index.json').read_text())), 13)

    def test_failed_render_preserves_previous_previews(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            old = out/'previews'/'old.png'
            old.parent.mkdir()
            old.write_bytes(b'previous export')

            def failing_cli(*args, **kwargs):
                if args[0] == 'eval':
                    return json.dumps([{'id':'1:1','name':'macOS / main','page':'macOS','w':100,'h':100}])
                raise RuntimeError('Renderer unavailable')

            with self.assertRaises(RuntimeError):
                verify.export(out/'unused.fig', out, failing_cli)
            self.assertEqual(old.read_bytes(), b'previous export')
            self.assertEqual(list(out.iterdir()), [old.parent])


if __name__ == '__main__':
    unittest.main()
