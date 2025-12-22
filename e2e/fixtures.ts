import { test as base, chromium, type BrowserContext, type Page } from '@playwright/test';
import path from 'path';
import { fileURLToPath } from 'url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const EXTENSION_PATH = path.join(__dirname, '../dist');
const EXTENSION_ID_PATTERN = /chrome-extension:\/\/([a-z]{32})/;

type ExtensionFixtures = {
  context: BrowserContext;
  extensionId: string;
  backgroundPage: Page;
};

export const test = base.extend<ExtensionFixtures>({
  context: async ({}, use) => {
    const userDataDir = '';
    const context = await chromium.launchPersistentContext(userDataDir, {
      headless: false,
      args: [
        `--disable-extensions-except=${EXTENSION_PATH}`,
        `--load-extension=${EXTENSION_PATH}`,
        '--no-first-run',
        '--no-default-browser-check',
        '--disable-default-apps',
      ],
    });
    
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
