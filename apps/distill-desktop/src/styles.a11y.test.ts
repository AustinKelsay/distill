/**
 * Static CSS accessibility token and reduced-motion contracts.
 */

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const stylesPath = join(dirname(fileURLToPath(import.meta.url)), "styles.css");
const css = readFileSync(stylesPath, "utf8");

/**
 * Parse a hex color into sRGB channels in 0..1.
 * @param hex - #rrggbb color
 */
function hexToRgb(hex: string): { r: number; g: number; b: number } {
  const match = /^#([0-9a-f]{6})$/i.exec(hex);
  if (!match) throw new Error(`invalid hex color: ${hex}`);
  const value = Number.parseInt(match[1], 16);
  return {
    r: ((value >> 16) & 255) / 255,
    g: ((value >> 8) & 255) / 255,
    b: (value & 255) / 255,
  };
}

/**
 * Convert one sRGB channel to linear light.
 * @param channel - 0..1 sRGB channel
 */
function toLinear(channel: number): number {
  return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
}

/**
 * Relative luminance for WCAG contrast math.
 * @param hex - #rrggbb color
 */
function luminance(hex: string): number {
  const { r, g, b } = hexToRgb(hex);
  return 0.2126 * toLinear(r) + 0.7152 * toLinear(g) + 0.0722 * toLinear(b);
}

/**
 * WCAG contrast ratio between two hex colors.
 * @param foreground - text color
 * @param background - background color
 */
function contrastRatio(foreground: string, background: string): number {
  const lighter = Math.max(luminance(foreground), luminance(background));
  const darker = Math.min(luminance(foreground), luminance(background));
  return (lighter + 0.05) / (darker + 0.05);
}

/**
 * Read a CSS custom property value from the stylesheet text.
 * @param name - custom property name including leading --
 */
function readToken(name: string): string {
  const match = new RegExp(`${name}:\\s*(#[0-9a-fA-F]{6})`).exec(css);
  if (!match) throw new Error(`missing token ${name}`);
  return match[1];
}

describe("renderer style accessibility tokens", () => {
  it("keeps body text and muted text above 4.5:1 on paper/panel", () => {
    const ink = readToken("--ink");
    const muted = readToken("--muted");
    const paper = readToken("--paper");
    const panel = readToken("--panel");
    const accent = readToken("--accent");
    const accentInk = readToken("--accent-ink");
    const disabledBg = readToken("--disabled-bg");
    const disabledInk = readToken("--disabled-ink");
    const danger = readToken("--danger");
    const focusRing = readToken("--focus-ring");

    expect(contrastRatio(ink, paper)).toBeGreaterThanOrEqual(4.5);
    expect(contrastRatio(ink, panel)).toBeGreaterThanOrEqual(4.5);
    expect(contrastRatio(muted, paper)).toBeGreaterThanOrEqual(4.5);
    expect(contrastRatio(accentInk, accent)).toBeGreaterThanOrEqual(4.5);
    expect(contrastRatio(disabledInk, disabledBg)).toBeGreaterThanOrEqual(4.5);
    expect(contrastRatio(danger, paper)).toBeGreaterThanOrEqual(4.5);
    expect(contrastRatio(focusRing, paper)).toBeGreaterThanOrEqual(3);
    expect(contrastRatio(focusRing, panel)).toBeGreaterThanOrEqual(3);
  });

  it("declares focus-visible, reduced-motion, rem sizing, and non-opacity-only disabled styles", () => {
    expect(css).toMatch(/:focus-visible/);
    expect(css).toMatch(/@media\s*\(prefers-reduced-motion:\s*reduce\)/);
    expect(css).toMatch(/font-size:\s*100%/);
    expect(css).toMatch(/max-width:\s*52rem/);
    expect(css).toMatch(/button:disabled\s*\{[^}]*background:\s*var\(--disabled-bg\)/s);
    expect(css).toMatch(/button:disabled\s*\{[^}]*opacity:\s*1/s);
  });
});
