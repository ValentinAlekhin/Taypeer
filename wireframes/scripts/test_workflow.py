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
