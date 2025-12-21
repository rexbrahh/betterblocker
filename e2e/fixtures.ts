import { test as base, chromium, type BrowserContext, type Page } from '@playwright/test';
import path from 'path';

const EXTENSION_PATH = path.join(__dirname, '../dist');
const EXTENSION_ID_PATTERN = /chrome-extension:\/\/([a-z]{32})/;

export const test = base.extend<{
  context: BrowserContext;
  extensionId: string;
  backgroundPage: Page;
}>({
  // eslint-disable-next-line no-empty-pattern
  context: async ({}, use) => {
    const context = await chromium.launchPersistentContext('', {
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
    await context.close();
  },

  backgroundPage: async ({ context }, use) => {
    let [bgPage] = context.backgroundPages();
    if (!bgPage) {
      bgPage = await context.waitForEvent('backgroundpage', { timeout: 10000 });
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
