import type { PresetRoute, RouteRecord } from "./api";

export interface StagedRoute extends PresetRoute {
  expectedRouteRevision: number;
}

export function createPipelineId(): string {
  const bytes = globalThis.crypto.getRandomValues(new Uint8Array(16));
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
  return [
    hex.slice(0, 8), hex.slice(8, 12), hex.slice(12, 16),
    hex.slice(16, 20), hex.slice(20),
  ].join("-");
}

export function stageRouteSelection(
  stagedRoutes: StagedRoute[],
  selection: StagedRoute,
): StagedRoute[] {
  const existingIndex = stagedRoutes.findIndex(
    (route) => route.destinationId === selection.destinationId,
  );
  if (existingIndex < 0) {
    return [...stagedRoutes, selection];
  }
  if (
    stagedRoutes[existingIndex].sourceId === selection.sourceId &&
    stagedRoutes[existingIndex].expectedRouteRevision === selection.expectedRouteRevision &&
    (stagedRoutes[existingIndex].action ?? "start") === (selection.action ?? "start")
  ) {
    return stagedRoutes;
  }
  return stagedRoutes.map((route, index) =>
    index === existingIndex ? selection : route,
  );
}

export function toggleRouteSelection(
  stagedRoutes: StagedRoute[],
  selection: StagedRoute,
): StagedRoute[] {
  const existing = stagedRoutes.find((route) =>
    route.sourceId === selection.sourceId &&
    route.destinationId === selection.destinationId);
  if (!existing) return stageRouteSelection(stagedRoutes, selection);
  if (existing.action === "stop") {
    return stagedRoutes.filter((route) => route.destinationId !== selection.destinationId);
  }
  return stageRouteSelection(stagedRoutes, { ...existing, action: "stop" });
}

export function snapshotPresetRoutes(
  selections: PresetRoute[],
  routes: RouteRecord[],
): StagedRoute[] {
  return selections.map((selection) => ({
    ...selection,
    expectedRouteRevision:
      routes.find((route) => route.destinationId === selection.destinationId)?.revision ?? 0,
  }));
}

export function hasStaleRouteSelection(
  selections: StagedRoute[],
  routes: RouteRecord[],
): boolean {
  return selections.some((selection) =>
    selection.expectedRouteRevision !==
      (routes.find((route) => route.destinationId === selection.destinationId)?.revision ?? 0));
}

export function matchesPresetRoutes(
  selections: PresetRoute[],
  saved: PresetRoute[],
): boolean {
  return selections.length === saved.length &&
    saved.every((route) => selections.some((selection) =>
      selection.sourceId === route.sourceId &&
      selection.destinationId === route.destinationId &&
      (selection.action ?? "start") === (route.action ?? "start")));
}
