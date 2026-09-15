"""Saved-FIG checks and transactional export of generated previews."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import tempfile

ROOT = Path(__file__).resolve().parents[1]
INDEX = "return figma.root.children.flatMap(p=>p.children.map(n=>({id:n.id,name:n.name,page:p.name,w:n.width,h:n.height})));"


def read_index(fig, cli):
    return json.loads(cli('eval', fig, '-c', INDEX, '--json'))


def write_json(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n')


def verify(fig, out, cli):
    out.mkdir(parents=True, exist_ok=True)
    index = read_index(fig, cli)
    result = json.loads(cli('eval', fig, '--stdin', '--json', input=(ROOT/'source/checks.js').read_text()))
    result.update(artboards=len(index), fig_sha256=hashlib.sha256(fig.read_bytes()).hexdigest())
    result['ok'] = bool(index) and not (result['overflow'] or result['narrow_text'] or result['design_contracts']['errors'])
    write_json(out/'screen-index.json', index)
    write_json(out/'qa.json', result)
    return result


def export(fig, out, cli):
    from PIL import Image, ImageDraw, ImageFont
    out.mkdir(parents=True, exist_ok=True)
    index = read_index(fig, cli)
    if not index:
        raise RuntimeError('No artboards to export.')
    staging = Path(tempfile.mkdtemp(prefix='previews-', dir=out))
    font = ImageFont.load_default()
    for face in ['/System/Library/Fonts/Supplemental/Arial.ttf', 'DejaVuSans.ttf']:
        try:
            font = ImageFont.truetype(face, 11)
            break
        except OSError:
            continue
    try:
        files = set()
        for node in index:
            name = node['name'].replace(' / ', '-').replace(' ', '-').lower()
            # Artboard names cannot escape the generated output directory.
            name = name.replace('/', '-').replace('\\', '-').replace('..', '-') + '.png'
            if name in files:
                raise RuntimeError('Duplicate export filename: ' + name)
            files.add(name)
            node['file'] = name
            cli('export', fig, '--node', node['id'], '-o', staging/name)
            with Image.open(staging/name) as check:
                check.verify()
        groups = ['macos', 'android', 'android-form', 'android-states', 'android-qa', 'android-flow',
                  'dialog', 'tabs', 'menus', 'states', 'qa']
        def category(node):
            return next((p for p in sorted(groups, key=len, reverse=True)
                         if node['file'].startswith(p + '-')), None)
        for prefix in groups:
            subset = [n for n in index if category(n) == prefix]
            if not subset:
                continue
            phones = prefix in ['android', 'android-form', 'android-qa']
            tw, th = (528, 328) if prefix == 'macos' else (195, 422) if phones else (392, 400)
            cols = 4 if phones else 3
            for start in range(0, len(subset), 12):
                chunk = subset[start:start+12]
                rows = (len(chunk)+cols-1)//cols
                sheet = Image.new('RGB', (cols*(tw+20)+20, rows*(th+42)+20), '#252525')
                draw = ImageDraw.Draw(sheet)
                for i, node in enumerate(chunk):
                    with Image.open(staging/node['file']) as original:
                        im = original.convert('RGB')
                    im.thumbnail((tw, th))
                    x, y = 20+i%cols*(tw+20), 20+i//cols*(th+42)
                    sheet.paste(im, (x, y))
                    draw.text((x, y+th+8), node['name'], fill='#dddddd', font=font)
                suffix = '' if start == 0 else f'-{start//12+1:02}'
                sheet.save(staging/(prefix+'-contact'+suffix+'.png'))
        write_json(staging/'index.json', index)
        final = out/'previews'
        previous = None
        if final.exists():
            previous = Path(tempfile.mkdtemp(prefix='previous-previews-', dir=out))
            previous.rmdir()
            os.replace(final, previous)
        try:
            os.replace(staging, final)
        except BaseException:
            if previous:
                os.replace(previous, final)
            raise
        if previous:
            shutil.rmtree(previous)
        write_json(out/'screen-index.json', [{k:v for k,v in n.items() if k!='file'} for n in index])
        return {'exported': len(index), 'directory': str(final), 'fig_sha256': hashlib.sha256(fig.read_bytes()).hexdigest()}
    finally:
        if staging.exists():
            shutil.rmtree(staging)
