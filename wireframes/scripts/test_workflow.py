"""Guard against losing saved/live edits and losing old previews on export failure."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import design
import verify


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


class ExportSafety(unittest.TestCase):
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
