import { test as base, chromium, type BrowserContext, type Page } from '@playwright/test';
import path from 'path';
import { fileURLToPath } from 'url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const EXTENSION_PATH = path.join(__dirname, '../dist');
const EXTENSION_ID_PATTERN = /chrome-extension:\/\/([a-z]{32})/;

const CHROME_PATH = process.env.E2E_CHROME_PATH;
const CHROME_CHANNEL = resolveChromeChannel(process.env.E2E_CHROME_CHANNEL);
const HEADLESS = resolveHeadless(process.env.E2E_HEADLESS);
const EXTRA_ARGS = parseExtraArgs(process.env.E2E_CHROME_ARGS);

type ExtensionFixtures = {
  context: BrowserContext;
  extensionId: string;
  backgroundPage: Page;
};

type ChromeChannel = 'chrome' | 'msedge' | 'chrome-beta' | 'chrome-dev' | 'chrome-canary';

function resolveChromeChannel(value?: string): ChromeChannel | undefined {
  if (!value) {
    return undefined;
  }
  switch (value) {
    case 'chrome':
    case 'msedge':
    case 'chrome-beta':
    case 'chrome-dev':
    case 'chrome-canary':
      return value;
    default:
      return undefined;
  }
}

function resolveHeadless(value?: string): boolean {
  if (!value) {
    return false;
  }
  const normalized = value.trim().toLowerCase();
  return normalized === '1' || normalized === 'true' || normalized === 'yes';
}

function parseExtraArgs(value?: string): string[] {
  if (!value) {
    return [];
  }
  const delimiter = value.includes(';') ? ';' : ',';
  return value
    .split(delimiter)
    .map((arg) => arg.trim())
    .filter((arg) => arg.length > 0);
}

export const test = base.extend<ExtensionFixtures>({
  context: async ({}, use) => {
    const userDataDir = '';
    const launchOptions: Parameters<typeof chromium.launchPersistentContext>[1] = {
      headless: HEADLESS,
      ignoreDefaultArgs: ['--disable-extensions'],
      args: [
        `--disable-extensions-except=${EXTENSION_PATH}`,
        `--load-extension=${EXTENSION_PATH}`,
        '--no-first-run',
        '--no-default-browser-check',
        '--disable-default-apps',
        ...EXTRA_ARGS,
      ],
    };

    if (CHROME_PATH) {
      launchOptions.executablePath = CHROME_PATH;
    } else if (CHROME_CHANNEL) {
      launchOptions.channel = CHROME_CHANNEL;
    }

    const context = await chromium.launchPersistentContext(userDataDir, launchOptions);
    
    await use(context);
    
    try {
      await context.close();
    } catch {
    }
  },

  backgroundPage: async ({ context }, use) => {
    let bgPage: Page | undefined;
    const pages = context.backgroundPages();
    if (pages.length > 0) {
      bgPage = pages[0];
    } else {
      bgPage = await context.waitForEvent('backgroundpage', { timeout: 15000 });
    }
    await use(bgPage);
  },

  extensionId: async ({ backgroundPage }, use) => {
    const url = backgroundPage.url();
    const match = url.match(EXTENSION_ID_PATTERN);
    const extensionId = match ? match[1] : '';
    await use(extensionId);
  },
});

export const expect = test.expect;
