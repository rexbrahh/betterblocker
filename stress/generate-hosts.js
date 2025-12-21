const fs = require('fs');
const path = require('path');

const SOURCE_FILE = path.resolve(__dirname, '../ultimate.txt');
const OUTPUT_FILE = path.resolve(__dirname, 'hosts.json');

function extractDomain(line) {
  const trimmed = line.trim();
  if (!trimmed || trimmed.startsWith('!') || trimmed.startsWith('[')) return null;

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
  const content = fs.readFileSync(SOURCE_FILE, 'utf8');
  const lines = content.split('\n');
  const domains = new Set();

  for (const line of lines) {
    const domain = extractDomain(line);
    if (domain && domain.includes('.') && !domain.includes('*')) {
      domains.add(domain);
    }
  }

  const uniqueDomains = Array.from(domains);
  fs.writeFileSync(OUTPUT_FILE, JSON.stringify(uniqueDomains, null, 2));

  console.log(`Generated ${OUTPUT_FILE}`);
  console.log(`Source lines: ${lines.length}`);
  console.log(`Unique domains: ${uniqueDomains.length}`);
} catch (err) {
  console.error(`Error: ${err.message}`);
}
