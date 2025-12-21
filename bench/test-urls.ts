export interface TestRequest {
  url: string;
  type: string;
  initiator: string | undefined;
}

const AD_DOMAINS = [
  'ads.example.com',
  'tracking.example.com',
  'analytics.test.com',
  'doubleclick.net',
  'googlesyndication.com',
  'googleadservices.com',
  'google-analytics.com',
  'adservice.google.com',
  'pagead2.googlesyndication.com',
  'cdn.ads.com',
  'metrics.example.com',
];

const CLEAN_DOMAINS = [
  'example.com',
  'google.com',
  'github.com',
  'stackoverflow.com',
  'reddit.com',
  'twitter.com',
  'facebook.com',
  'amazon.com',
  'wikipedia.org',
  'mozilla.org',
];

const PATHS = [
  '/',
  '/index.html',
  '/assets/main.js',
  '/api/v1/data',
  '/images/logo.png',
  '/styles/app.css',
  '/ads/banner.gif',
  '/tracking/pixel.gif',
  '/analytics.js',
  '/beacon.js',
];

const REQUEST_TYPES = [
  'main_frame',
  'sub_frame',
  'script',
  'stylesheet',
  'image',
  'xmlhttprequest',
  'font',
  'ping',
];

const DEFAULT_SEED = 0xc0ffee;

function createRng(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state * 1664525 + 1013904223) >>> 0;
    return state / 0x100000000;
  };
}

function pick<T>(items: readonly T[], rand: () => number): T {
  return items[Math.floor(rand() * items.length)]!;
}

export function generateTestRequests(count: number, seed = DEFAULT_SEED): TestRequest[] {
  const requests: TestRequest[] = [];
  const rand = createRng(seed);

  for (let i = 0; i < count; i++) {
    const isAdRequest = rand() < 0.3;
    const domain = pick(isAdRequest ? AD_DOMAINS : CLEAN_DOMAINS, rand);
    const path = pick(PATHS, rand);
    const type = pick(REQUEST_TYPES, rand);

    const initiatorDomain = pick(CLEAN_DOMAINS, rand);
    const isThirdParty = rand() < 0.6;
    const initiator = isThirdParty ? `https://${initiatorDomain}/` : `https://${domain}/`;

    requests.push({
      url: `https://${domain}${path}`,
      type,
      initiator: type === 'main_frame' ? undefined : initiator,
    });
  }

  return requests;
}

export function generateRealisticMix(): TestRequest[] {
  const requests: TestRequest[] = [];

  requests.push({ url: 'https://example.com/', type: 'main_frame', initiator: undefined });
  requests.push({ url: 'https://example.com/app.js', type: 'script', initiator: 'https://example.com/' });
  requests.push({ url: 'https://example.com/style.css', type: 'stylesheet', initiator: 'https://example.com/' });

  requests.push({ url: 'https://ads.example.com/banner.js', type: 'script', initiator: 'https://example.com/' });
  requests.push({ url: 'https://doubleclick.net/ads/show', type: 'xmlhttprequest', initiator: 'https://example.com/' });
  requests.push({ url: 'https://google-analytics.com/collect', type: 'ping', initiator: 'https://example.com/' });

  requests.push({ url: 'https://cdn.example.com/lib.js', type: 'script', initiator: 'https://example.com/' });
  requests.push({ url: 'https://fonts.googleapis.com/css', type: 'stylesheet', initiator: 'https://example.com/' });

  requests.push({ url: 'https://tracking.example.com/pixel.gif', type: 'image', initiator: 'https://example.com/' });
  requests.push({ url: 'https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js', type: 'script', initiator: 'https://example.com/' });

  return requests;
}
