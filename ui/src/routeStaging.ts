export interface StagedRoute {
  sourceId: string;
  destinationId: string;
  expectedRouteRevision: number;
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
    stagedRoutes[existingIndex].expectedRouteRevision ===
      selection.expectedRouteRevision
  ) {
    return stagedRoutes;
  }
  return stagedRoutes.map((route, index) =>
    index === existingIndex ? selection : route,
  );
}
