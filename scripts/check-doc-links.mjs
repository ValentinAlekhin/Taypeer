import { readFileSync, realpathSync, statSync } from 'node:fs';
import path from 'node:path';
import MarkdownIt from 'markdown-it';
import GithubSlugger from 'github-slugger';

const markdown = new MarkdownIt({ html: true });

// Parse Markdown rather than treating examples in code fences as links.
export function parseDocument(text) {
  const tokens = markdown.parse(text, {});
  const slugger = new GithubSlugger();
  const anchors = new Set();
  const links = [];
  function visit(token) {
    if (token.type === 'link_open') links.push(token.attrGet('href'));
    if (token.type === 'image') links.push(token.attrGet('src'));
    if (token.type === 'html_inline' || token.type === 'html_block') {
      for (const match of token.content.matchAll(/\b(?:id|name)=["']([^"']+)["']/g)) anchors.add(match[1]);
    }
    for (const child of token.children ?? []) visit(child);
  }
  for (let i = 0; i < tokens.length; i++) {
    if (tokens[i].type === 'heading_open') {
      const content = (tokens[i + 1].children ?? [])
        .filter(t => ['text', 'code_inline', 'image'].includes(t.type))
        .map(t => t.content).join('');
      anchors.add(slugger.slug(content));
    }
    visit(tokens[i]);
  }
  return { anchors, links };
}

export function createLinkChecker(root) {
  const boundary = realpathSync(root);
  const cache = new Map();
  function document(file) {
    if (!cache.has(file)) cache.set(file, parseDocument(readFileSync(file, 'utf8')));
    return cache.get(file);
  }
  function check(reference, from) {
    if (!reference || /^(?:[a-z][a-z\d+.-]*:|\/\/)/i.test(reference)) return null;
    try {
      const hash = reference.indexOf('#');
      const rawPath = (hash < 0 ? reference : reference.slice(0, hash)).split('?')[0];
      const anchor = hash < 0 ? '' : decodeURIComponent(reference.slice(hash + 1));
      const target = rawPath ? path.resolve(path.dirname(from), decodeURIComponent(rawPath)) : from;
      const resolved = realpathSync(target);
      const relative = path.relative(boundary, resolved);
      if (relative === '..' || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)) {
        return `outside repository: ${reference}`;
      }
      if (anchor && statSync(resolved).isFile() && /\.md$/i.test(resolved) && !document(resolved).anchors.has(anchor)) {
        return `missing anchor: ${reference}`;
      }
      return null;
    } catch (error) {
      return `invalid local reference ${reference}: ${error.code ?? error.message}`;
    }
  }
  return { check, document };
}
