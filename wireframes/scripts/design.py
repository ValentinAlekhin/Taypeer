#!/usr/bin/env python3
"""Build, inspect, export and safely replace the local Taypeer design artifact."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
from datetime import datetime, timezone

ROOT = Path(__file__).resolve().parents[1]
BUILD = ROOT / 'build'
FIG = ROOT / 'taypeer.fig'
CANDIDATE = BUILD / 'taypeer.fig'
STATE = ROOT / 'source' / 'published.json'
MANIFEST = BUILD / 'candidate.json'


def run_cli(*args, input=None):
    command = ['openpencil', *map(str, args)]
    if shutil.which('rtk'):
        command = ['rtk', 'proxy', *command]
    result = subprocess.run(command, input=input, text=True, capture_output=True, timeout=60)
    if result.returncode:
        raise RuntimeError((result.stderr + result.stdout).strip())
    return result.stdout


def digest(data):
    return hashlib.sha256(data).hexdigest()


def file_hash(path):
    return digest(path.read_bytes())


def json_hash(data):
    return digest(json.dumps(data, ensure_ascii=False, sort_keys=True, separators=(',', ':')).encode())


def write_json(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=path.parent, mode='w', encoding='utf-8', delete=False) as stream:
        json.dump(data, stream, ensure_ascii=False, indent=2)
        stream.write('\n')
        temporary = stream.name
    os.replace(temporary, path)


def scene(file=None, document_id=None):
    args = ['eval']
    if file is not None:
        args.append(file)
    if document_id is not None:
        args.extend(['--document-id', document_id])
    args.extend(['--stdin', '--json'])
    return json.loads(run_cli(*args, input=(ROOT / 'source/snapshot.js').read_text()))


def input_hash():
    paths = ['source/compact.js', 'source/desktop.js', 'source/checks.js', 'source/snapshot.js',
             'assets/lucide-base.fig', 'assets/lucide-extra.json',
             'assets/demo-invitation.json',
             'scripts/design.py', 'scripts/verify.py']
    return json_hash({name: file_hash(ROOT / name) for name in paths})


def build():
    BUILD.mkdir(exist_ok=True)
    # Capture the saved artifact before generation; publish checks it again.
    base_hash = file_hash(FIG)
    inputs = input_hash()
    icons = json.loads((ROOT / 'assets/lucide-extra.json').read_text())
    invitation = json.loads((ROOT / 'assets/demo-invitation.json').read_text())
    code = ('const EXTRA_ICONS = ' + json.dumps(icons) + ';\n'
            + 'const DEMO_INVITATION = ' + json.dumps(invitation) + ';\n'
            + (ROOT / 'source/compact.js').read_text() + '\n'
            + (ROOT / 'source/desktop.js').read_text())
    temporary = BUILD / 'candidate-writing.fig'
    try:
        result = json.loads(run_cli('eval', ROOT / 'assets/lucide-base.fig', '-o', temporary,
                                    '--stdin', '--json', input=code).split('\nWritten to ')[0])
        if input_hash() != inputs:
            raise RuntimeError('Sources changed during build; rebuild before publishing.')
        os.replace(temporary, CANDIDATE)
        write_json(MANIFEST, {'base_file_sha256': base_hash, 'inputs_sha256': inputs,
                             'candidate_sha256': file_hash(CANDIDATE), 'summary': result})
    finally:
        temporary.unlink(missing_ok=True)
    print(json.dumps({'candidate': str(CANDIDATE), **result}, ensure_ascii=False))


def check_candidate():
    if not CANDIDATE.is_file() or not MANIFEST.is_file():
        raise RuntimeError('No build candidate. Run build first.')
    manifest = json.loads(MANIFEST.read_text())
    if manifest['inputs_sha256'] != input_hash() or manifest['candidate_sha256'] != file_hash(CANDIDATE):
        raise RuntimeError('Candidate or sources changed. Run build again.')
    return manifest


def assert_publish_safe(*, current_file, expected_file, current_scene, live_scene,
                        candidate_scene, baseline_scene):
    if current_file != expected_file:
        raise RuntimeError('Saved FIG changed since build. Preserve the changes and rebuild.')
    if live_scene != current_scene:
        raise RuntimeError('OpenPencil contains unsaved changes. Preserve and reconcile them before publish.')
    if baseline_scene is None:
        if candidate_scene != current_scene:
            raise RuntimeError('No baseline: first build must reproduce the saved FIG before publish.')
    elif current_scene != baseline_scene and candidate_scene != current_scene:
        raise RuntimeError('Saved manual edits differ from the last published scene. Port them into the source first.')


def publish(document_id):
    from verify import verify
    manifest = check_candidate()
    report = verify(CANDIDATE, BUILD, run_cli)
    if not report['ok']:
        raise RuntimeError('Candidate failed design checks; see build/qa.json.')
    docs = json.loads(run_cli('documents', '--json'))
    docs = docs.get('documents', []) if isinstance(docs, dict) else docs
    doc = next((d for d in docs if d['id'] == document_id), None)
    if not doc or not doc.get('path') or Path(doc['path']).resolve() != FIG.resolve():
        raise RuntimeError('The selected OpenPencil document is not this project\'s taypeer.fig.')
    current_hash = file_hash(FIG)
    current_scene = json_hash(scene(FIG))
    candidate_scene = json_hash(scene(CANDIDATE))
    baseline = json.loads(STATE.read_text()) if STATE.exists() else {}
    live_scene = json_hash(scene(document_id=document_id))
    assert_publish_safe(current_file=current_hash, expected_file=manifest['base_file_sha256'],
                        current_scene=current_scene, live_scene=live_scene,
                        candidate_scene=candidate_scene, baseline_scene=baseline.get('scene_sha256'))
    check_candidate()
    if file_hash(FIG) != current_hash:
        raise RuntimeError('Saved FIG changed during publish checks; stopped.')
    backup = BUILD / 'backups' / (datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S%fZ') + '.fig')
    backup.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(FIG, backup)
    temporary = BUILD / 'publish-writing.fig'
    try:
        shutil.copy2(CANDIDATE, temporary)
        os.replace(temporary, FIG)
    finally:
        temporary.unlink(missing_ok=True)
    write_json(STATE, {'format': 1, 'file_sha256': file_hash(FIG),
                       'scene_sha256': candidate_scene, 'inputs_sha256': input_hash()})
    print(json.dumps({'published': str(FIG), 'backup': str(backup),
                      'next': 'Reload this saved file in OpenPencil before any app save.'}, ensure_ascii=False))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    sub.add_parser('build', help='Create build/taypeer.fig; leave the delivered FIG untouched')
    for name in ['verify', 'export']:
        item = sub.add_parser(name)
        item.add_argument('--published', action='store_true', help='Use delivered taypeer.fig instead of the candidate')
    item = sub.add_parser('publish', help='Validate and replace the FIG after checking disk and live document')
    item.add_argument('--document-id', required=True, help='Obtain from openpencil documents --json')
    sub.add_parser('status', help='Report saved artifact, baseline and candidate hashes')
    args = parser.parse_args()
    if args.command == 'build':
        build()
    elif args.command in ['verify', 'export']:
        from verify import verify, export
        if not args.published:
            check_candidate()
        target = FIG if args.published else CANDIDATE
        if args.command == 'verify':
            report = verify(target, BUILD, run_cli)
            print(json.dumps({'ok': report['ok'], 'artboards': report['artboards'],
                              'overflow': len(report['overflow']), 'narrow_text': len(report['narrow_text']),
                              'design_contract_errors': len(report['design_contracts']['errors']),
                              'report': str(BUILD/'qa.json')}, ensure_ascii=False))
            if not report['ok']:
                raise RuntimeError('Design checks failed; see build/qa.json.')
        else:
            print(json.dumps(export(target, BUILD, run_cli), ensure_ascii=False))
    elif args.command == 'publish':
        publish(args.document_id)
    else:
        data = {'saved_sha256': file_hash(FIG), 'saved_scene_sha256': json_hash(scene(FIG)),
                'baseline': json.loads(STATE.read_text()) if STATE.exists() else None,
                'candidate': json.loads(MANIFEST.read_text()) if MANIFEST.exists() else None}
        print(json.dumps(data, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    try:
        main()
    except (RuntimeError, OSError, ValueError, subprocess.TimeoutExpired) as error:
        print(f'Error: {error}', file=sys.stderr)
        raise SystemExit(1)
