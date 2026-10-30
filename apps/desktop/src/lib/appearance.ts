/** Fonts and density, per user (localStorage), applied as data-* attributes on <html>. */
import { useEffect, useState } from "react";

export type UiFont = "inter" | "geist" | "system";
export type MonoFont = "jetbrains" | "geist" | "system";
export type Density = "compact" | "comfortable";
export interface Appearance {
  uiFont: UiFont;
  monoFont: MonoFont;
  density: Density;
  /** stop queries that run longer than this (0 = never) */
  queryTimeoutSec: number;
}

const KEY = "rh-appearance";
export const DEFAULT_APPEARANCE: Appearance = { uiFont: "inter", monoFont: "jetbrains", density: "compact", queryTimeoutSec: 0 };

export function loadAppearance(): Appearance {
  try {
    return { ...DEFAULT_APPEARANCE, ...JSON.parse(localStorage.getItem(KEY) ?? "{}") };
  } catch {
    return DEFAULT_APPEARANCE;
  }
}

export function applyAppearance(a: Appearance) {
  const d = document.documentElement.dataset;
  d.uiFont = a.uiFont;
  d.monoFont = a.monoFont;
  d.density = a.density;
}

export function saveAppearance(a: Appearance) {
  try {
    localStorage.setItem(KEY, JSON.stringify(a));
  } catch {
    /* applied for this session only */
  }
  applyAppearance(a);
  window.dispatchEvent(new CustomEvent("rh-appearance", { detail: a }));
}

export function useAppearance(): Appearance {
  const [a, setA] = useState(loadAppearance);
  useEffect(() => {
    const on = (e: Event) => setA((e as CustomEvent<Appearance>).detail);
    window.addEventListener("rh-appearance", on);
    return () => window.removeEventListener("rh-appearance", on);
  }, []);
  return a;
}

/** Grid row height for the current density. */
export const rowHeight = (d: Density) => (d === "compact" ? 22 : 26);
