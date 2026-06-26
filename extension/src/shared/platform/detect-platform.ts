import { BUILTIN_PLATFORMS, type PlatformDefinition } from './selectors';

export type PlatformId = PlatformDefinition['id'];

export function detectPlatform(url: string): PlatformDefinition | null {
  for (const platform of BUILTIN_PLATFORMS) {
    for (const pattern of platform.match) {
      const prefix = pattern.replace(/\/\*$/, '');
      if (url.startsWith(prefix)) return platform;
    }
  }
  return null;
}

export function getPlatformById(id: PlatformId | string): PlatformDefinition | null {
  return BUILTIN_PLATFORMS.find((p) => p.id === id) ?? null;
}

export function isSupportedPlatform(url: string): boolean {
  return detectPlatform(url) !== null;
}
