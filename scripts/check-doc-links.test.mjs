import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createLinkChecker, parseDocument } from './check-doc-links.mjs';

test('Markdown links include reference links but exclude code examples', () => {
  const parsed = parseDocument('[first](a.md) and [second][doc]\n\n[doc]: <b file.md>\n\n```md\n[example](absent.md)\n```');
  assert.deepEqual(parsed.links, ['a.md', 'b%20file.md']);
});

test('Cyrillic headings, duplicate headings and explicit anchors resolve', () => {
  const parsed = parseDocument('# Модель **данных**\n# Модель данных\n<a id="stable"></a>');
  assert.deepEqual([...parsed.anchors], ['модель-данных', 'модель-данных-1', 'stable']);
});

test('Broken files and anchors fail; encoded local paths work', () => {
  const root = mkdtempSync(path.join(os.tmpdir(), 'taypeer-links-'));
  try {
    const from = path.join(root, 'README.md');
    writeFileSync(from, '# Home\n');
    writeFileSync(path.join(root, 'data model.md'), '# Данные\n');
    const checker = createLinkChecker(root);
    assert.equal(checker.check('data%20model.md#данные', from), null);
    assert.match(checker.check('data%20model.md#missing', from), /missing anchor/);
    assert.match(checker.check('missing.md', from), /invalid local reference/);
    assert.match(checker.check('../', from), /outside repository/);
    assert.equal(checker.check('https://example.invalid/doc#anchor', from), null);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
