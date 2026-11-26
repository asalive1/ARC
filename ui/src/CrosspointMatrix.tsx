import { useEffect, useRef, useState, type MouseEvent as ReactMouseEvent, type UIEvent } from "react";
import { Alert, Box, Button, Tooltip, Typography } from "@mui/material";

import type {
  DestinationRecord,
  RouteRecord,
  SourceRecord,
} from "./api";

const MAX_PORTS = 64;
const ROW_HEADER_WIDTH = 200;
const CELL_WIDTH = 52;
const ROW_HEIGHT = 44;
const HEADER_HEIGHT = 112;
const OVERSCAN = 2;

export interface VisibleMatrixRange {
  start: number;
  end: number;
}

export function isObservedRunningRoute(
  route: RouteRecord | undefined,
  sourceId: string,
): boolean {
  return (
    route !== undefined &&
    route.observedState === "running" &&
    observedSourceId(route) === sourceId
  );
}

function observedSourceId(route: RouteRecord | undefined): string | undefined {
  if (!route) {
    return undefined;
  }
  if (route.observedSourceId !== undefined) {
    return route.observedSourceId ?? undefined;
  }
  return route.observedState === "running" ? route.sourceId : undefined;
}

export function getDesiredOutputCount(
  routes: RouteRecord[],
  sourceId: string,
): number {
  return routes.filter(
    (route) => route.desiredState === "desired" && route.sourceId === sourceId,
  ).length;
}

export function getVisibleMatrixRange(
  offset: number,
  viewportSize: number,
  itemCount: number,
  itemSize: number,
  overscan = OVERSCAN,
): VisibleMatrixRange {
  if (itemCount <= 0 || itemSize <= 0) {
    return { start: 0, end: 0 };
  }

  const safeOffset = Math.max(0, offset);
  const start = Math.min(
    itemCount,
    Math.max(0, Math.floor(safeOffset / itemSize) - overscan),
  );
  const end = Math.min(
    itemCount,
    Math.max(
      start,
      Math.ceil((safeOffset + Math.max(0, viewportSize)) / itemSize) +
        overscan,
    ),
  );
  return { start, end };
}

interface CrosspointMatrixProps {
  sources: SourceRecord[];
  destinations: DestinationRecord[];
  routes: RouteRecord[];
  routesLoading: boolean;
  routesError: string;
  selectedSourceId: string;
  selectedDestinationId: string;
  selections?: { sourceId: string; destinationId: string }[];
  onSelect: (sourceId: string, destinationId: string) => void;
  onContextMenu?: (
    event: ReactMouseEvent<HTMLButtonElement>,
    sourceId: string,
    destinationId: string,
  ) => void;
}

export function CrosspointMatrix({
  sources,
  destinations,
  routes,
  routesLoading,
  routesError,
  selectedSourceId,
  selectedDestinationId,
  selections,
  onSelect,
  onContextMenu,
}: CrosspointMatrixProps) {
  const viewportRef = useRef<HTMLDivElement>(null);
  const [viewport, setViewport] = useState({
    top: 0,
    left: 0,
    width: 0,
    height: 0,
  });
  const visibleSources = sources.slice(0, MAX_PORTS);
  const visibleDestinations = destinations.slice(0, MAX_PORTS);
  const hasPorts = visibleSources.length > 0 && visibleDestinations.length > 0;
  const routesByDestination = new Map(
    routes.map((route) => [route.destinationId, route]),
  );
  const desiredRouteCount = routes.filter(
    (route) => route.desiredState === "desired",
  ).length;
  const totalWidth =
    ROW_HEADER_WIDTH + visibleDestinations.length * CELL_WIDTH;
  const totalHeight =
    HEADER_HEIGHT + visibleSources.length * ROW_HEIGHT;

  useEffect(() => {
    if (!hasPorts) {
      return;
    }
    const node = viewportRef.current;
    if (!node) {
      return;
    }

    const updateSize = () => {
      setViewport((current) => ({
        ...current,
        width: node.clientWidth,
        height: node.clientHeight,
      }));
    };
    updateSize();
    const observer = new ResizeObserver(updateSize);
    observer.observe(node);
    return () => observer.disconnect();
  }, [hasPorts]);

  const rowRange = getVisibleMatrixRange(
    viewport.top,
    Math.max(0, viewport.height - HEADER_HEIGHT),
    visibleSources.length,
    ROW_HEIGHT,
  );
  const columnRange = getVisibleMatrixRange(
    viewport.left,
    Math.max(0, viewport.width - ROW_HEADER_WIDTH),
    visibleDestinations.length,
    CELL_WIDTH,
  );
  const visibleColumnCount = columnRange.end - columnRange.start;
  const rowColumns = [
    `${ROW_HEADER_WIDTH}px`,
    `${columnRange.start * CELL_WIDTH}px`,
    ...Array.from({ length: visibleColumnCount }, () => `${CELL_WIDTH}px`),
    `${(visibleDestinations.length - columnRange.end) * CELL_WIDTH}px`,
  ].join(" ");

  function updateScrollPosition(
    event: UIEvent<HTMLDivElement>,
  ): void {
    const node = event.currentTarget;
    setViewport((current) => ({
      ...current,
      top: node.scrollTop,
      left: node.scrollLeft,
    }));
  }

  if (!hasPorts) {
    return (
      <Box
        role="status"
        sx={{
          p: 2,
          border: 1,
          borderColor: "divider",
          borderRadius: 1,
          color: "text.secondary",
        }}
      >
        Add at least one input and one output to use the crosspoint matrix.
      </Box>
    );
  }

  return (
    <Box>
      {(sources.length > MAX_PORTS || destinations.length > MAX_PORTS) && (
        <Typography color="warning.main" variant="body2" sx={{ mb: 1 }}>
          The matrix shows the first 64 inputs and first 64 outputs. The
          catalog currently exceeds the 64×64 matrix limit.
        </Typography>
      )}
      <Box
        ref={viewportRef}
        role="grid"
        aria-label="Routing crosspoint matrix"
        aria-rowcount={visibleSources.length + 1}
        aria-colcount={visibleDestinations.length + 1}
        onScroll={updateScrollPosition}
        sx={{
          height: 420,
          maxWidth: "100%",
          overflow: "auto",
          border: 1,
          borderColor: "divider",
          borderRadius: 1,
          bgcolor: "background.default",
        }}
      >
        <Box sx={{ width: totalWidth, height: totalHeight }}>
          <Box
            role="row"
            aria-rowindex={1}
            sx={{
              display: "grid",
              gridTemplateColumns: rowColumns,
              position: "sticky",
              top: 0,
              zIndex: 2,
              width: totalWidth,
              height: HEADER_HEIGHT,
              bgcolor: "background.paper",
              borderBottom: 1,
              borderColor: "divider",
            }}
          >
            <Box
              role="columnheader"
              aria-colindex={1}
              sx={{
                position: "sticky",
                left: 0,
                zIndex: 3,
                display: "flex",
                alignItems: "flex-end",
                p: 1,
                bgcolor: "background.paper",
                borderRight: 1,
                borderColor: "divider",
              }}
            >
              <Typography variant="caption" color="text.secondary">
                INPUT ↓ / OUTPUT →
              </Typography>
            </Box>
            <Box aria-hidden="true" />
            {visibleDestinations
              .slice(columnRange.start, columnRange.end)
              .map((destination, visibleIndex) => {
                const index = columnRange.start + visibleIndex;
                return (
                  <Tooltip
                    key={destination.id}
                    title={`${index + 1}. ${destination.friendlyName}`}
                  >
                    <Box
                      role="columnheader"
                      aria-label={destination.friendlyName}
                      aria-colindex={index + 2}
                      sx={{
                        display: "flex",
                        justifyContent: "center",
                        alignItems: "flex-end",
                        width: CELL_WIDTH,
                        height: HEADER_HEIGHT,
                        pb: 1,
                        overflow: "hidden",
                        borderRight: 1,
                        borderColor: "divider",
                      }}
                    >
                      <Typography
                        variant="caption"
                        noWrap
                        sx={{
                          writingMode: "vertical-rl",
                          transform: "rotate(180deg)",
                          maxHeight: HEADER_HEIGHT - 12,
                          textOverflow: "ellipsis",
                        }}
                      >
                        {destination.friendlyName}
                      </Typography>
                    </Box>
                  </Tooltip>
                );
              })}
            <Box aria-hidden="true" />
          </Box>
          <Box
            aria-hidden="true"
            sx={{ height: rowRange.start * ROW_HEIGHT }}
          />
          {visibleSources
            .slice(rowRange.start, rowRange.end)
            .map((source, visibleIndex) => {
              const rowIndex = rowRange.start + visibleIndex;
              const desiredOutputCount = getDesiredOutputCount(
                routes,
                source.id,
              );
              return (
                <Box
                  key={source.id}
                  role="row"
                  aria-rowindex={rowIndex + 2}
                  sx={{
                    display: "grid",
                    gridTemplateColumns: rowColumns,
                    width: totalWidth,
                    height: ROW_HEIGHT,
                    borderBottom: 1,
                    borderColor: "divider",
                  }}
                >
                  <Tooltip
                    title={`${rowIndex + 1}. ${source.friendlyName}${
                      desiredOutputCount > 1
                        ? `; ${desiredOutputCount} desired outputs`
                        : ""
                    }`}
                  >
                    <Box
                      role="rowheader"
                      aria-label={`${source.friendlyName}${
                        desiredOutputCount > 1
                          ? `, ${desiredOutputCount} desired outputs`
                          : ""
                      }`}
                      aria-colindex={1}
                      sx={{
                        position: "sticky",
                        left: 0,
                        zIndex: 1,
                        display: "flex",
                        flexDirection: "column",
                        alignItems: "flex-start",
                        justifyContent: "center",
                        px: 1,
                        bgcolor: "background.paper",
                        borderRight: 1,
                        borderColor: "divider",
                        overflow: "hidden",
                      }}
                    >
                      <Typography variant="body2" noWrap>
                        {source.friendlyName}
                      </Typography>
                      {desiredOutputCount > 1 && (
                        <Typography
                          variant="caption"
                          color="text.secondary"
                          noWrap
                        >
                          {desiredOutputCount} desired outputs
                        </Typography>
                      )}
                    </Box>
                  </Tooltip>
                  <Box aria-hidden="true" />
                  {visibleDestinations
                    .slice(columnRange.start, columnRange.end)
                    .map((destination, visibleIndex) => {
                      const selected = selections
                        ? selections.some(
                            (selection) =>
                              selection.sourceId === source.id &&
                              selection.destinationId === destination.id,
                          )
                        : source.id === selectedSourceId &&
                          destination.id === selectedDestinationId;
                      const destinationRoute = routesByDestination.get(
                        destination.id,
                      );
                      const desired =
                        destinationRoute?.desiredState === "desired" &&
                        destinationRoute.sourceId === source.id;
                      const observed = isObservedRunningRoute(
                        destinationRoute,
                        source.id,
                      );
                      const observedId = observedSourceId(destinationRoute);
                      const observedSource = sources.find(
                        (candidate) => candidate.id === observedId,
                      );
                      const routeDescription = destinationRoute
                        ? `Desired route revision ${destinationRoute.revision} is ${destinationRoute.desiredState}. Observed ${destinationRoute.observedState}${observedSource ? ` from ${observedSource.friendlyName}` : ""}.`
                        : "There is no saved route for this output.";
                      const cellDescription = [
                        desired ? "Desired route." : "",
                        observed ? "Worker-observed route is running." : "",
                        destinationRoute?.desiredState === "desired" &&
                        !desired
                          ? "This output has a desired route from another input."
                          : "",
                        routeDescription,
                      ]
                        .filter(Boolean)
                        .join(" ");
                      return (
                        <Box
                          key={destination.id}
                          role="gridcell"
                          aria-colindex={columnRange.start + visibleIndex + 2}
                          aria-selected={selected}
                          sx={{
                            display: "flex",
                            justifyContent: "center",
                            alignItems: "center",
                            width: CELL_WIDTH,
                            height: ROW_HEIGHT,
                          }}
                        >
                          <Tooltip disableInteractive title={`${source.friendlyName} → ${destination.friendlyName}. ${cellDescription}`}>
                            <Button
                              aria-label={`Select ${source.friendlyName} to ${destination.friendlyName}. ${cellDescription}`}
                              aria-pressed={selected}
                              aria-haspopup={onContextMenu ? "menu" : undefined}
                              onClick={() =>
                                onSelect(source.id, destination.id)
                              }
                              onContextMenu={
                                onContextMenu
                                  ? (event) => onContextMenu(event, source.id, destination.id)
                                  : undefined
                              }
                              onKeyDown={(event) => {
                                if (onContextMenu &&
                                    (event.key === "ContextMenu" ||
                                      (event.shiftKey && event.key === "F10"))) {
                                  event.preventDefault();
                                  event.currentTarget.dispatchEvent(
                                    new MouseEvent("contextmenu", {
                                      bubbles: true,
                                      clientX: event.currentTarget.getBoundingClientRect().left,
                                      clientY: event.currentTarget.getBoundingClientRect().bottom,
                                    }),
                                  );
                                }
                              }}
                              size="small"
                              color={
                                selected
                                  ? "primary"
                                  : observed
                                    ? "success"
                                    : desired
                                      ? "warning"
                                      : "inherit"
                              }
                              variant={
                                selected || desired || observed
                                  ? "contained"
                                  : "outlined"
                              }
                              sx={{
                                minWidth: 34,
                                width: 34,
                                height: 32,
                                p: 0,
                                borderRadius: 1,
                              }}
                            >
                              <Box
                                component="span"
                                aria-hidden="true"
                                sx={{
                                  width: selected ? 12 : 8,
                                  height: selected ? 12 : 8,
                                  borderRadius: "50%",
                                  bgcolor: selected
                                    ? "common.white"
                                    : observed
                                      ? "common.white"
                                      : "text.disabled",
                                }}
                              />
                            </Button>
                          </Tooltip>
                        </Box>
                      );
                    })}
                  <Box aria-hidden="true" />
                </Box>
              );
            })}
          <Box
            aria-hidden="true"
            sx={{
              height:
                (visibleSources.length - rowRange.end) * ROW_HEIGHT,
            }}
          />
        </Box>
      </Box>
      <Typography variant="caption" color="text.secondary" sx={{ mt: 1 }}>
        {visibleSources.length} inputs × {visibleDestinations.length} outputs
        {" · "}
        {routesError
          ? "Saved route status unavailable."
          : routesLoading
            ? "Loading saved route status."
            : `${desiredRouteCount} saved desired routes.`}{" "}
        Blue marks the selected crosspoint, green the observed running route,
        and amber a desired route not yet observed running. An input may feed
        multiple outputs, with an independent route outcome for each output.
      </Typography>
      {routesError && (
        <Alert severity="error" sx={{ mt: 1 }}>
          Saved route state could not be loaded: {routesError}. Crosspoint
          selection remains available, but saved route status is unavailable.
        </Alert>
      )}
    </Box>
  );
}
