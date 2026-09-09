import { readdirSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { LikeC4 } from 'likec4';
import { createLinkChecker } from './check-doc-links.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
const checker = createLinkChecker(root);
const errors = [];
const docs = ['README.md', 'AGENTS.md', 'CONTEXT.md', 'spec.md', 'spikes/README.md',
  '.agents/skills/taypeer-architecture/SKILL.md',
  '.agents/skills/taypeer-rust/SKILL.md',
  ...readdirSync(path.join(root, 'docs')).filter(f => f.endsWith('.md')).map(f => `docs/${f}`)];

for (const name of docs) {
  const file = path.join(root, name);
  for (const reference of checker.document(file).links) {
    const error = checker.check(reference, file);
    if (error) errors.push(`${name}: ${error}`);
  }
}

const likec4 = await LikeC4.fromWorkspace(path.join(root, 'architecture'), { watch: false });
try {
  if (likec4.hasErrors()) throw new Error('LikeC4 model is invalid; run pnpm exec likec4 validate architecture');
  const model = (await likec4.computedModel()).$data;
  const statuses = new Set(['scaffold', 'partial', 'planned', 'external', 'conceptual']);
  for (const element of Object.values(model.elements)) {
    if (element.tags.filter(t => statuses.has(t)).length !== 1) errors.push(`${element.id}: expected one implementation/category tag`);
    if (element.tags.some(t => t === 'scaffold' || t === 'partial') && !element.metadata?.source) errors.push(`${element.id}: implemented portions need an existing source`);
    for (const key of ['source', 'contract', 'evidence', 'glossary']) {
      const value = element.metadata?.[key];
      for (const reference of value == null ? [] : Array.isArray(value) ? value : [value]) {
        const error = checker.check(reference, path.join(root, 'README.md'));
        if (error) errors.push(`${element.id}.${key}: ${error}`);
      }
    }
  }
  const relationships = new Set();
  for (const relation of Object.values(model.relations)) {
    const signature = JSON.stringify([relation.source, relation.target, relation.kind, relation.title]);
    if (relationships.has(signature)) errors.push(`duplicate model relationship: ${signature}`);
    relationships.add(signature);
  }
  for (const view of Object.values(model.views)) {
    if (view.nodes.length === 0) errors.push(`${view.id}: empty view`);
  }
  if (errors.length) {
    for (const error of errors) console.error(error);
    process.exitCode = 1;
  } else {
    console.log(`Architecture: ${Object.keys(model.elements).length} elements, ${Object.keys(model.views).length} views; ${docs.length} Markdown documents and model references checked.`);
  }
} finally {
  await likec4.dispose();
}
