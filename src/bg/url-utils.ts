/**
 * Fast URL parsing utilities for the hot path
 * 
 * These functions avoid using the URL constructor which is expensive.
 * All operations work directly on strings with minimal allocations.
 */

import { SchemeMask } from '../shared/types.js';
import { hashToken } from '../shared/hash.js';

// =============================================================================
// Scheme Extraction
// =============================================================================

/**
 * Fast scheme extraction without URL constructor.
 * Returns the scheme mask or 0 if unknown.
 */
export function fastExtractScheme(url: string): SchemeMask | 0 {
  // Check common schemes first (most URLs are http/https)
  if (url.length < 5) return 0;
  
  const c0 = url.charCodeAt(0) | 0x20; // Lowercase
  
  if (c0 === 0x68) { // 'h'
    if (url.length >= 8 && url.substring(0, 8).toLowerCase() === 'https://') {
      return SchemeMask.HTTPS;
    }
    if (url.length >= 7 && url.substring(0, 7).toLowerCase() === 'http://') {
      return SchemeMask.HTTP;
    }
  } else if (c0 === 0x77) { // 'w'
    if (url.length >= 6 && url.substring(0, 6).toLowerCase() === 'wss://') {
      return SchemeMask.WSS;
    }
    if (url.length >= 5 && url.substring(0, 5).toLowerCase() === 'ws://') {
      return SchemeMask.WS;
    }
  } else if (c0 === 0x64) { // 'd'
    if (url.length >= 5 && url.substring(0, 5).toLowerCase() === 'data:') {
      return SchemeMask.DATA;
    }
  } else if (c0 === 0x66) { // 'f'
    if (url.length >= 6 && url.substring(0, 6).toLowerCase() === 'ftp://') {
      return SchemeMask.FTP;
    }
  }
  
  return 0;
}

/**
 * Get the scheme end position (position after "://").
 */
export function getSchemeEnd(url: string): number {
  const colonPos = url.indexOf(':');
  if (colonPos === -1) return 0;
  
  // Check for "://"
  if (url.length > colonPos + 2 &&
      url.charCodeAt(colonPos + 1) === 0x2f && // '/'
      url.charCodeAt(colonPos + 2) === 0x2f) { // '/'
    return colonPos + 3;
  }
  
  // Data URLs use ":" not "://"
  if (url.substring(0, colonPos).toLowerCase() === 'data') {
    return colonPos + 1;
  }
  
  return 0;
}

// =============================================================================
// Host Extraction
// =============================================================================

/**
 * Fast host extraction without URL constructor.
 * Returns empty string if host cannot be extracted.
 */
export function fastExtractHost(url: string): string {
  const schemeEnd = getSchemeEnd(url);
  if (schemeEnd === 0) return '';
  
  // Find host end (first of: '/', '?', '#', ':' for port, or end of string)
  let hostEnd = url.length;
  for (let i = schemeEnd; i < url.length; i++) {
    const c = url.charCodeAt(i);
    if (c === 0x2f || c === 0x3f || c === 0x23 || c === 0x3a) { // '/', '?', '#', ':'
      hostEnd = i;
      break;
    }
  }
  
  const host = url.substring(schemeEnd, hostEnd).toLowerCase();
  
  // Handle userinfo (user:pass@host)
  const atPos = host.indexOf('@');
  if (atPos !== -1) {
    return host.substring(atPos + 1);
  }
  
  return host;
}

/**
 * Extract host with port if present.
 */
export function fastExtractHostWithPort(url: string): string {
  const schemeEnd = getSchemeEnd(url);
  if (schemeEnd === 0) return '';
  
  // Find host end (first of: '/', '?', '#', or end of string)
  let hostEnd = url.length;
  for (let i = schemeEnd; i < url.length; i++) {
    const c = url.charCodeAt(i);
    if (c === 0x2f || c === 0x3f || c === 0x23) { // '/', '?', '#'
      hostEnd = i;
      break;
    }
  }
  
  const hostWithPort = url.substring(schemeEnd, hostEnd).toLowerCase();
  
  // Handle userinfo
  const atPos = hostWithPort.indexOf('@');
  if (atPos !== -1) {
    return hostWithPort.substring(atPos + 1);
  }
  
  return hostWithPort;
}

// =============================================================================
// Path Extraction
// =============================================================================

/**
 * Extract the path portion of a URL (after host, before query/fragment).
 */
export function fastExtractPath(url: string): string {
  const schemeEnd = getSchemeEnd(url);
  if (schemeEnd === 0) return '';
  
  // Find path start (first '/' after host)
  let pathStart = -1;
  for (let i = schemeEnd; i < url.length; i++) {
    const c = url.charCodeAt(i);
    if (c === 0x2f) { // '/'
      pathStart = i;
      break;
    }
    if (c === 0x3f || c === 0x23) { // '?', '#'
      return '/'; // No path, just query/fragment
    }
  }
  
  if (pathStart === -1) return '/';
  
  // Find path end
  let pathEnd = url.length;
  for (let i = pathStart; i < url.length; i++) {
    const c = url.charCodeAt(i);
    if (c === 0x3f || c === 0x23) { // '?', '#'
      pathEnd = i;
      break;
    }
  }
  
  return url.substring(pathStart, pathEnd);
}

// =============================================================================
// URL Tokenization for Pattern Matching
// =============================================================================

// Reusable buffer for tokens to avoid allocation
const TOKEN_BUFFER = new Uint32Array(32);
const MIN_TOKEN_LEN = 3;
const MAX_TOKENS = 32;

/**
 * Check if a character is alphanumeric.
 */
function isAlnum(c: number): boolean {
  return (c >= 0x30 && c <= 0x39) || // 0-9
         (c >= 0x41 && c <= 0x5a) || // A-Z
         (c >= 0x61 && c <= 0x7a);   // a-z
}

/**
 * Tokenize a URL for pattern matching.
 * Returns hashed tokens in a reusable buffer.
 * 
 * Tokens are alphanumeric runs of at least MIN_TOKEN_LEN characters.
 * Returns [buffer, count] where buffer contains the hashes.
 */
export function tokenizeUrl(url: string): [Uint32Array, number] {
  let tokenCount = 0;
  let tokenStart = -1;
  
  // Start after scheme to avoid matching protocol tokens
  const schemeEnd = getSchemeEnd(url);
  const start = schemeEnd > 0 ? schemeEnd : 0;
  
  for (let i = start; i <= url.length && tokenCount < MAX_TOKENS; i++) {
    const c = i < url.length ? url.charCodeAt(i) : 0;
    const isAlphaNum = isAlnum(c);
    
    if (isAlphaNum) {
      if (tokenStart === -1) {
        tokenStart = i;
      }
    } else {
      if (tokenStart !== -1) {
        const len = i - tokenStart;
        if (len >= MIN_TOKEN_LEN) {
          const token = url.substring(tokenStart, i).toLowerCase();
          TOKEN_BUFFER[tokenCount++] = hashToken(token);
        }
        tokenStart = -1;
      }
    }
  }
  
  return [TOKEN_BUFFER, tokenCount];
}

/**
 * Extract tokens as strings (for debugging/testing).
 */
export function extractTokenStrings(url: string): string[] {
  const tokens: string[] = [];
  let tokenStart = -1;
  
  const schemeEnd = getSchemeEnd(url);
  const start = schemeEnd > 0 ? schemeEnd : 0;
  
  for (let i = start; i <= url.length && tokens.length < MAX_TOKENS; i++) {
    const c = i < url.length ? url.charCodeAt(i) : 0;
    const isAlphaNum = isAlnum(c);
    
    if (isAlphaNum) {
      if (tokenStart === -1) {
        tokenStart = i;
      }
    } else {
      if (tokenStart !== -1) {
        const len = i - tokenStart;
        if (len >= MIN_TOKEN_LEN) {
          tokens.push(url.substring(tokenStart, i).toLowerCase());
        }
        tokenStart = -1;
      }
    }
  }
  
  return tokens;
}

// =============================================================================
// URL Normalization
// =============================================================================

/**
 * Normalize a URL for consistent matching.
 * - Lowercases scheme and host
 * - Removes default ports
 * - Removes trailing slashes on path-less URLs
 */
export function normalizeUrl(url: string): string {
  const schemeEnd = getSchemeEnd(url);
  if (schemeEnd === 0) return url;
  
  // Find host end
  let hostEnd = url.length;
  for (let i = schemeEnd; i < url.length; i++) {
    const c = url.charCodeAt(i);
    if (c === 0x2f || c === 0x3f || c === 0x23) {
      hostEnd = i;
      break;
    }
  }
  
  const scheme = url.substring(0, schemeEnd).toLowerCase();
  let hostPart = url.substring(schemeEnd, hostEnd).toLowerCase();
  const rest = url.substring(hostEnd);
  
  // Remove default ports
  if (scheme === 'http://' && hostPart.endsWith(':80')) {
    hostPart = hostPart.slice(0, -3);
  } else if (scheme === 'https://' && hostPart.endsWith(':443')) {
    hostPart = hostPart.slice(0, -4);
  }
  
  return scheme + hostPart + rest;
}

// =============================================================================
// ABP Boundary Check (for ^ separator)
// =============================================================================

/**
 * Check if a character is an ABP separator (boundary character).
 * ABP ^ matches: end of string, or any non-alphanumeric non-% character.
 */
export function isBoundaryChar(c: number): boolean {
  // End of string is a boundary
  if (c === 0) return true;
  
  // Alphanumeric is not a boundary
  if (isAlnum(c)) return false;
  
  // % is not a boundary (URL encoding)
  if (c === 0x25) return false;
  
  // Everything else is a boundary
  return true;
}

/**
 * Check if position in string is at a boundary.
 */
export function isAtBoundary(str: string, pos: number): boolean {
  if (pos >= str.length) return true;
  return isBoundaryChar(str.charCodeAt(pos));
}

// =============================================================================
// Query Parameter Handling (for removeparam)
// =============================================================================

/**
 * Parse query string into key-value pairs.
 * Does not decode values (we compare encoded).
 */
export function parseQueryParams(url: string): Map<string, string[]> {
  const params = new Map<string, string[]>();
  
  const qPos = url.indexOf('?');
  if (qPos === -1) return params;
  
  // Find end of query (before fragment)
  let qEnd = url.indexOf('#', qPos);
  if (qEnd === -1) qEnd = url.length;
  
  const query = url.substring(qPos + 1, qEnd);
  if (query.length === 0) return params;
  
  for (const pair of query.split('&')) {
    const eqPos = pair.indexOf('=');
    const key = eqPos === -1 ? pair : pair.substring(0, eqPos);
    const value = eqPos === -1 ? '' : pair.substring(eqPos + 1);
    
    if (key.length === 0) continue;
    
    const existing = params.get(key);
    if (existing) {
      existing.push(value);
    } else {
      params.set(key, [value]);
    }
  }
  
  return params;
}

/**
 * Remove specified parameters from a URL.
 * Returns the modified URL, or the original if no changes.
 */
export function removeQueryParams(url: string, keysToRemove: Set<string>): string {
  const qPos = url.indexOf('?');
  if (qPos === -1) return url;
  
  // Find fragment
  const hashPos = url.indexOf('#', qPos);
  const fragment = hashPos === -1 ? '' : url.substring(hashPos);
  const qEnd = hashPos === -1 ? url.length : hashPos;
  
  const query = url.substring(qPos + 1, qEnd);
  if (query.length === 0) return url;
  
  const pairs = query.split('&');
  const kept: string[] = [];
  let changed = false;
  
  for (const pair of pairs) {
    const eqPos = pair.indexOf('=');
    const key = eqPos === -1 ? pair : pair.substring(0, eqPos);
    
    if (keysToRemove.has(key)) {
      changed = true;
    } else {
      kept.push(pair);
    }
  }
  
  if (!changed) return url;
  
  const base = url.substring(0, qPos);
  if (kept.length === 0) {
    return base + fragment;
  }
  return base + '?' + kept.join('&') + fragment;
}

// =============================================================================
// Hostname position in URL (for hostname anchor matching)
// =============================================================================

/**
 * Get the start and end positions of the hostname in a URL.
 * Returns [start, end] or [-1, -1] if not found.
 */
export function getHostPosition(url: string): [number, number] {
  const schemeEnd = getSchemeEnd(url);
  if (schemeEnd === 0) return [-1, -1];
  
  // Skip userinfo if present
  let hostStart = schemeEnd;
  for (let i = schemeEnd; i < url.length; i++) {
    if (url.charCodeAt(i) === 0x40) { // '@'
      hostStart = i + 1;
      break;
    }
    if (url.charCodeAt(i) === 0x2f) { // '/'
      break;
    }
  }
  
  // Find host end
  let hostEnd = url.length;
  for (let i = hostStart; i < url.length; i++) {
    const c = url.charCodeAt(i);
    if (c === 0x2f || c === 0x3f || c === 0x23 || c === 0x3a) {
      hostEnd = i;
      break;
    }
  }
  
  return [hostStart, hostEnd];
}
