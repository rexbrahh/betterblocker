/**
 * Core Matching Engine
 * 
 * This is the hot path - every request goes through here.
 * Performance is critical: no allocations, minimal branching.
 * 
 * Decision order (per uBO precedence spec):
 * A0: Trusted site bypass
 * A1: Dynamic filtering (matrix)
 * A2: removeparam modifiers
 * A3: Static network exceptions and blocks
 * A4: redirect vs redirect-rule semantics
 * A5: Redirect directive application
 */

import type {
  RequestContext,
  MatchResult,
} from '../shared/types.js';
import {
  RuleAction,
  RuleFlags,
  MatchDecision,
  PartyMask,
  ALL_PARTIES,
  ALL_SCHEMES,
} from '../shared/types.js';
import { hashDomain } from '../shared/hash.js';
import { getETLD1, walkHostSuffixes } from '../shared/psl.js';
import {
  Snapshot,
  decodePostingList,
  type TokenEntry,
} from '../shared/snapshot/loader.js';
import { PatternOp } from '../shared/snapshot/format.js';
import {
  fastExtractHost,
  fastExtractScheme,
  tokenizeUrl,
  isAtBoundary,
  getHostPosition,
} from './url-utils.js';

// =============================================================================
// Matcher State
// =============================================================================

/** Currently active snapshot */
let activeSnapshot: Snapshot | null = null;

/** Trusted sites (user whitelist) */
const trustedSites = new Set<string>();

/** Decision cache: (cacheKey) -> MatchResult */
const decisionCache = new Map<string, MatchResult>();
const DECISION_CACHE_MAX = 8192;

// =============================================================================
// Initialization
// =============================================================================

/**
 * Initialize the matcher with a snapshot.
 */
export function initMatcher(snapshot: Snapshot): void {
  activeSnapshot = snapshot;
  decisionCache.clear();
}

/**
 * Get the current snapshot (for testing/debugging).
 */
export function getActiveSnapshot(): Snapshot | null {
  return activeSnapshot;
}

/**
 * Add a site to the trusted list (bypass all blocking).
 */
export function addTrustedSite(site: string): void {
  trustedSites.add(site.toLowerCase());
  // Clear cache entries for this site
  for (const key of decisionCache.keys()) {
    if (key.startsWith(site)) {
      decisionCache.delete(key);
    }
  }
}

/**
 * Remove a site from the trusted list.
 */
export function removeTrustedSite(site: string): void {
  trustedSites.delete(site.toLowerCase());
}

/**
 * Check if a site is trusted.
 */
export function isTrustedSite(siteETLD1: string): boolean {
  return trustedSites.has(siteETLD1);
}

// =============================================================================
// Main Match Function
// =============================================================================

/**
 * Match a request and return the decision.
 * This is the hot path entry point.
 */
export function matchRequest(ctx: RequestContext): MatchResult {
  // A0: Trusted site bypass
  if (trustedSites.has(ctx.siteETLD1)) {
    return { decision: MatchDecision.ALLOW, ruleId: -1, listId: -1 };
  }

  // Check decision cache
  const cacheKey = buildCacheKey(ctx);
  const cached = decisionCache.get(cacheKey);
  if (cached) {
    return cached;
  }

  // No snapshot loaded
  if (!activeSnapshot) {
    return { decision: MatchDecision.ALLOW, ruleId: -1, listId: -1 };
  }

  // A1: Dynamic filtering would go here (not implemented in MVP)
  
  // A2: removeparam would go here (not implemented in MVP)
  
  // A3: Static network filtering
  const result = matchStaticFilters(ctx, activeSnapshot);
  
  // Cache the result
  if (decisionCache.size >= DECISION_CACHE_MAX) {
    // Simple eviction: clear half the cache
    const keys = Array.from(decisionCache.keys());
    for (let i = 0; i < keys.length / 2; i++) {
      decisionCache.delete(keys[i]!);
    }
  }
  decisionCache.set(cacheKey, result);

  return result;
}

/**
 * Build a cache key for the decision cache.
 */
function buildCacheKey(ctx: RequestContext): string {
  // Key format: siteETLD1|reqETLD1|type|party|scheme|urlHash
  // We use a simple string key for now; could optimize to numeric key later
  return `${ctx.siteETLD1}|${ctx.reqETLD1}|${ctx.type}|${ctx.isThirdParty ? '3' : '1'}|${ctx.scheme}|${simpleUrlHash(ctx.url)}`;
}

/**
 * Simple URL hash for cache key (not cryptographic).
 */
function simpleUrlHash(url: string): number {
  let hash = 0;
  for (let i = 0; i < url.length; i++) {
    hash = ((hash << 5) - hash + url.charCodeAt(i)) | 0;
  }
  return hash >>> 0;
}

// =============================================================================
// Static Filter Matching (A3)
// =============================================================================

interface MatchCandidate {
  ruleId: number;
  action: RuleAction;
  isImportant: boolean;
  priority: number;
}

/**
 * Match against static filters in the snapshot.
 */
function matchStaticFilters(ctx: RequestContext, snapshot: Snapshot): MatchResult {
  const candidates: MatchCandidate[] = [];
  
  // Step 1: Check domain sets (host-only rules)
  matchDomainSets(ctx, snapshot, candidates);
  
  // Step 2: Check token-indexed URL rules
  matchTokenRules(ctx, snapshot, candidates);
  
  // Step 3: Apply precedence logic
  return applyPrecedence(candidates, snapshot);
}

/**
 * Match against domain hash sets.
 * Uses suffix-walk: check full host, then parent, etc.
 */
function matchDomainSets(
  ctx: RequestContext,
  snapshot: Snapshot,
  candidates: MatchCandidate[]
): void {
  const allowSet = snapshot.domainAllowSet;
  const blockSet = snapshot.domainBlockSet;
  
  // Walk suffixes from most specific to least
  for (const suffix of walkHostSuffixes(ctx.reqHost)) {
    const hash = hashDomain(suffix);
    
    // Check allow set
    const allowRuleId = allowSet.lookup(hash);
    if (allowRuleId !== -1) {
      candidates.push({
        ruleId: allowRuleId,
        action: RuleAction.ALLOW,
        isImportant: false,
        priority: 0,
      });
    }
    
    // Check block set
    const blockRuleId = blockSet.lookup(hash);
    if (blockRuleId !== -1) {
      // Check if this rule is important
      const flags = snapshot.rules.flags[blockRuleId] ?? 0;
      const isImportant = (flags & RuleFlags.IMPORTANT) !== 0;
      
      candidates.push({
        ruleId: blockRuleId,
        action: RuleAction.BLOCK,
        isImportant,
        priority: 0,
      });
    }
  }
}

/**
 * Match against token-indexed URL pattern rules.
 */
function matchTokenRules(
  ctx: RequestContext,
  snapshot: Snapshot,
  candidates: MatchCandidate[]
): void {
  const tokenDict = snapshot.tokenDict;
  const postings = snapshot.tokenPostings;
  const rules = snapshot.rules;
  const patternPool = snapshot.patternPool;
  
  // Tokenize the URL
  const [tokenHashes, tokenCount] = tokenizeUrl(ctx.url);
  if (tokenCount === 0) return;
  
  // Find the rarest token(s) to minimize candidate set
  let bestEntry: TokenEntry | null = null;
  let bestCount = Infinity;
  
  for (let i = 0; i < tokenCount; i++) {
    const hash = tokenHashes[i]!;
    const entry = tokenDict.lookup(hash);
    if (entry && entry.ruleCount < bestCount) {
      bestEntry = entry;
      bestCount = entry.ruleCount;
    }
  }
  
  if (!bestEntry) return;
  
  // Decode the posting list
  const ruleIds = decodePostingList(postings, bestEntry.postingsOffset, bestEntry.ruleCount);
  
  // Verify each candidate
  for (let i = 0; i < ruleIds.length; i++) {
    const ruleId = ruleIds[i]!;
    
    // Quick option checks first (before expensive pattern match)
    if (!checkRuleOptions(ruleId, ctx, rules)) {
      continue;
    }
    
    // Check domain constraints
    if (!checkDomainConstraints(ruleId, ctx, snapshot)) {
      continue;
    }
    
    // Pattern verification
    const patternId = rules.patternId[ruleId];
    if (patternId !== undefined && patternId !== 0xffffffff) {
      const pattern = patternPool.getPattern(patternId);
      if (pattern && !verifyPattern(ctx.url, pattern, patternPool, snapshot)) {
        continue;
      }
    }
    
    // Rule matches!
    const action = rules.action[ruleId] as RuleAction | undefined;
    const flags = rules.flags[ruleId] ?? 0;
    const priority = rules.priority[ruleId] ?? 0;
    
    if (action !== undefined) {
      candidates.push({
        ruleId,
        action,
        isImportant: (flags & RuleFlags.IMPORTANT) !== 0,
        priority,
      });
    }
  }
}

/**
 * Check if a rule's options match the request context.
 */
function checkRuleOptions(
  ruleId: number,
  ctx: RequestContext,
  rules: Snapshot['rules']
): boolean {
  // Type mask
  const typeMask = rules.typeMask[ruleId] ?? 0;
  if (typeMask !== 0 && (typeMask & ctx.type) === 0) {
    return false;
  }
  
  // Party mask
  const partyMask = rules.partyMask[ruleId] ?? ALL_PARTIES;
  const requestParty = ctx.isThirdParty ? PartyMask.THIRD_PARTY : PartyMask.FIRST_PARTY;
  if ((partyMask & requestParty) === 0) {
    return false;
  }
  
  // Scheme mask
  const schemeMask = rules.schemeMask[ruleId] ?? ALL_SCHEMES;
  if (ctx.scheme !== 0 && (schemeMask & ctx.scheme) === 0) {
    return false;
  }
  
  return true;
}

/**
 * Check domain constraints ($domain=).
 */
function checkDomainConstraints(
  ruleId: number,
  ctx: RequestContext,
  snapshot: Snapshot
): boolean {
  const constraintOff = snapshot.rules.domainConstraintOff[ruleId];
  if (constraintOff === undefined || constraintOff === 0xffffffff) {
    return true; // No constraints
  }
  
  const constraints = snapshot.domainConstraints;
  const view = new DataView(constraints.buffer, constraints.byteOffset + constraintOff);
  
  const includeCount = view.getUint16(0, true);
  const excludeCount = view.getUint16(2, true);
  
  let pos = 4;
  const siteHash = hashDomain(ctx.siteETLD1);
  
  // Check include list (if non-empty, site must match one)
  if (includeCount > 0) {
    let found = false;
    for (let i = 0; i < includeCount; i++) {
      const lo = view.getUint32(pos, true);
      const hi = view.getUint32(pos + 4, true);
      pos += 8;
      
      if (lo === siteHash.lo && hi === siteHash.hi) {
        found = true;
        break;
      }
    }
    if (!found) return false;
  } else {
    // Skip include entries (there are none, but we already read the count)
    pos += includeCount * 8;
  }
  
  // Check exclude list (site must NOT match any)
  for (let i = 0; i < excludeCount; i++) {
    const lo = view.getUint32(pos, true);
    const hi = view.getUint32(pos + 4, true);
    pos += 8;
    
    if (lo === siteHash.lo && hi === siteHash.hi) {
      return false; // Excluded
    }
  }
  
  return true;
}

/**
 * Verify a URL against a compiled pattern program.
 */
function verifyPattern(
  url: string,
  pattern: ReturnType<Snapshot['patternPool']['getPattern']>,
  patternPool: Snapshot['patternPool'],
  snapshot: Snapshot
): boolean {
  if (!pattern) return false;
  
  const program = patternPool.getProgram(pattern);
  const urlLower = url.toLowerCase();
  let urlPos = 0;
  let progPos = 0;
  
  while (progPos < program.length) {
    const op = program[progPos]!;
    progPos++;
    
    switch (op) {
      case PatternOp.FIND_LIT: {
        // Read string offset and length
        const strOff = 
          program[progPos]! |
          (program[progPos + 1]! << 8) |
          (program[progPos + 2]! << 16) |
          (program[progPos + 3]! << 24);
        const strLen = program[progPos + 4]! | (program[progPos + 5]! << 8);
        progPos += 6;
        
        const literal = snapshot.getString(strOff, strLen).toLowerCase();
        const foundPos = urlLower.indexOf(literal, urlPos);
        if (foundPos === -1) {
          return false;
        }
        urlPos = foundPos + literal.length;
        break;
      }
      
      case PatternOp.ASSERT_START:
        if (urlPos !== 0) return false;
        break;
      
      case PatternOp.ASSERT_END:
        if (urlPos !== url.length) return false;
        break;
      
      case PatternOp.ASSERT_BOUNDARY:
        if (!isAtBoundary(url, urlPos)) return false;
        break;
      
      case PatternOp.SKIP_ANY:
        // Wildcard - just continue, next FIND_LIT will search
        break;
      
      case PatternOp.HOST_ANCHOR: {
        // Verify the match is within the hostname portion
        const [hostStart, hostEnd] = getHostPosition(url);
        if (hostStart === -1) return false;
        
        // For hostname anchor (||), the pattern must match at start of host
        // or after a dot in the host
        const hostHashLo = pattern.hostHashLo;
        const hostHashHi = pattern.hostHashHi;
        
        if (hostHashLo !== 0 || hostHashHi !== 0) {
          // Check if request host matches the anchor host
          const reqHost = fastExtractHost(url);
          
          // Walk suffixes to check for match
          let hostMatches = false;
          for (const suffix of walkHostSuffixes(reqHost)) {
            const suffixHash = hashDomain(suffix);
            if (suffixHash.lo === hostHashLo && suffixHash.hi === hostHashHi) {
              hostMatches = true;
              break;
            }
          }
          
          if (!hostMatches) return false;
        }
        
        // Position must be at or before host end
        if (urlPos > hostEnd) return false;
        break;
      }
      
      case PatternOp.DONE:
        return true;
      
      default:
        // Unknown opcode - fail safe
        return false;
    }
  }
  
  return true;
}

// =============================================================================
// Precedence Logic (A4, A5)
// =============================================================================

/**
 * Apply uBO precedence rules to determine final decision.
 * 
 * Order:
 * 1. IMPORTANT BLOCK wins over everything
 * 2. ALLOW (exception) wins over normal BLOCK
 * 3. BLOCK
 * 4. Default: ALLOW
 */
function applyPrecedence(
  candidates: MatchCandidate[],
  snapshot: Snapshot
): MatchResult {
  if (candidates.length === 0) {
    return { decision: MatchDecision.ALLOW, ruleId: -1, listId: -1 };
  }
  
  let bestImportantBlock: MatchCandidate | null = null;
  let bestAllow: MatchCandidate | null = null;
  let bestBlock: MatchCandidate | null = null;
  let bestRedirect: MatchCandidate | null = null;
  
  for (const c of candidates) {
    switch (c.action) {
      case RuleAction.BLOCK:
        if (c.isImportant) {
          if (!bestImportantBlock || c.priority > bestImportantBlock.priority) {
            bestImportantBlock = c;
          }
        } else {
          if (!bestBlock || c.priority > bestBlock.priority) {
            bestBlock = c;
          }
        }
        break;
      
      case RuleAction.ALLOW:
        if (!bestAllow || c.priority > bestAllow.priority) {
          bestAllow = c;
        }
        break;
      
      case RuleAction.REDIRECT_DIRECTIVE:
        if (!bestRedirect || c.priority > bestRedirect.priority) {
          bestRedirect = c;
        }
        break;
    }
  }
  
  // 1. IMPORTANT BLOCK wins (ignores exceptions)
  if (bestImportantBlock) {
    const listId = snapshot.rules.listId[bestImportantBlock.ruleId] ?? 0;
    
    // Check for redirect directive
    if (bestRedirect) {
      const redirectUrl = getRedirectUrl(bestRedirect.ruleId, snapshot);
      if (redirectUrl) {
        return {
          decision: MatchDecision.REDIRECT,
          ruleId: bestImportantBlock.ruleId,
          listId,
          redirectUrl,
        };
      }
    }
    
    return {
      decision: MatchDecision.BLOCK,
      ruleId: bestImportantBlock.ruleId,
      listId,
    };
  }
  
  // 2. ALLOW exception overrides normal block
  if (bestAllow && bestBlock) {
    const listId = snapshot.rules.listId[bestAllow.ruleId] ?? 0;
    return {
      decision: MatchDecision.ALLOW,
      ruleId: bestAllow.ruleId,
      listId,
    };
  }
  
  // 3. Normal BLOCK (with possible redirect)
  if (bestBlock) {
    const listId = snapshot.rules.listId[bestBlock.ruleId] ?? 0;
    
    if (bestRedirect) {
      const redirectUrl = getRedirectUrl(bestRedirect.ruleId, snapshot);
      if (redirectUrl) {
        return {
          decision: MatchDecision.REDIRECT,
          ruleId: bestBlock.ruleId,
          listId,
          redirectUrl,
        };
      }
    }
    
    return {
      decision: MatchDecision.BLOCK,
      ruleId: bestBlock.ruleId,
      listId,
    };
  }
  
  // 4. ALLOW (explicit or default)
  if (bestAllow) {
    const listId = snapshot.rules.listId[bestAllow.ruleId] ?? 0;
    return {
      decision: MatchDecision.ALLOW,
      ruleId: bestAllow.ruleId,
      listId,
    };
  }
  
  // Default: allow
  return { decision: MatchDecision.ALLOW, ruleId: -1, listId: -1 };
}

/**
 * Get the redirect URL for a redirect directive.
 */
function getRedirectUrl(ruleId: number, snapshot: Snapshot): string | undefined {
  // The optionId for REDIRECT_DIRECTIVE points to the resource index
  const optionId = snapshot.rules.optionId[ruleId];
  if (optionId === undefined || optionId === 0xffffffff) {
    return undefined;
  }
  
  // Look up in redirect resources section
  const section = snapshot.sections.get(0x0009); // REDIRECT_RESOURCES
  if (!section) return undefined;
  
  // Read resource path from the section
  // Format: u32 resourceCount, then entries
  const resourceCount = snapshot.view.getUint32(section.offset, true);
  if (optionId >= resourceCount) return undefined;
  
  const entryOffset = section.offset + 4 + optionId * 20;
  const pathStrOff = snapshot.view.getUint32(entryOffset + 8, true);
  const pathStrLen = snapshot.view.getUint32(entryOffset + 12, true);
  
  const resourcePath = snapshot.getString(pathStrOff, pathStrLen);
  
  // Return full extension URL
  // In browser context this would be chrome.runtime.getURL(resourcePath)
  // For now return the path; the caller can resolve it
  return resourcePath;
}

// =============================================================================
// Context Building
// =============================================================================

/**
 * Build a RequestContext from webRequest details.
 */
export function buildRequestContext(
  url: string,
  type: number,
  tabId: number,
  frameId: number,
  requestId: string,
  initiatorUrl?: string
): RequestContext {
  const reqHost = fastExtractHost(url);
  const reqETLD1 = getETLD1(reqHost);
  const scheme = fastExtractScheme(url);
  
  let siteHost = '';
  let siteETLD1 = '';
  
  if (initiatorUrl) {
    siteHost = fastExtractHost(initiatorUrl);
    siteETLD1 = getETLD1(siteHost);
  } else if (type === 64) { // MAIN_FRAME
    siteHost = reqHost;
    siteETLD1 = reqETLD1;
  }
  
  const isThirdParty = reqETLD1 !== siteETLD1;
  
  return {
    url,
    reqHost,
    reqETLD1,
    siteHost,
    siteETLD1,
    isThirdParty,
    type,
    scheme: scheme || 0,
    tabId,
    frameId,
    requestId,
  };
}

// =============================================================================
// Cache Management
// =============================================================================

/**
 * Clear the decision cache.
 */
export function clearDecisionCache(): void {
  decisionCache.clear();
}

/**
 * Get decision cache stats (for debugging).
 */
export function getDecisionCacheStats(): { size: number; maxSize: number } {
  return {
    size: decisionCache.size,
    maxSize: DECISION_CACHE_MAX,
  };
}
