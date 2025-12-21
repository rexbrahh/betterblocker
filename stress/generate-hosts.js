import fs from 'node:fs';
import { fileURLToPath } from 'node:url';

const DEFAULT_INPUT = fileURLToPath(new URL('../ultimate.txt', import.meta.url));
const OUTPUT_FILE = fileURLToPath(new URL('./hosts.json', import.meta.url));

function extractDomain(line) {
  const trimmed = line.trim();
  if (!trimmed || trimmed.startsWith('!') || trimmed.startsWith('[') || trimmed.startsWith('@@')) return null;

  if (trimmed.startsWith('||')) {
    const parts = trimmed.slice(2).split(/[\^/]/);
    return parts[0];
  }

  if (trimmed.startsWith('0.0.0.0 ') || trimmed.startsWith('127.0.0.1 ')) {
    return trimmed.split(/\s+/)[1];
  }

  return null;
}

try {
  const inputFiles = process.argv.slice(2);
  const sources = inputFiles.length ? inputFiles : [DEFAULT_INPUT];
  const domains = new Set();
  let totalLines = 0;

  for (const source of sources) {
    const content = fs.readFileSync(source, 'utf8');
    const lines = content.split('\n');
    totalLines += lines.length;

    for (const line of lines) {
      const domain = extractDomain(line);
      if (domain && domain.includes('.') && !domain.includes('*')) {
        domains.add(domain);
      }
    }
  }

  const uniqueDomains = Array.from(domains);
  fs.writeFileSync(OUTPUT_FILE, JSON.stringify(uniqueDomains, null, 2));

  console.log(`Generated ${OUTPUT_FILE}`);
  console.log(`Source files: ${sources.length}`);
  console.log(`Source lines: ${totalLines}`);
  console.log(`Unique domains: ${uniqueDomains.length}`);
} catch (err) {
  console.error(`Error: ${err.message}`);
}
