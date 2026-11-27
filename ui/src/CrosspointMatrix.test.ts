import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import {
  CrosspointMatrix,
  getDesiredOutputCount,
  getVisibleMatrixRange,
  isObservedRunningRoute,
} from "./CrosspointMatrix";
import type { DestinationRecord, RouteRecord, SourceRecord } from "./api";
import { snapshotPresetRoutes } from "./routeStaging";
import { DestinationOutcome, RoutingWorkspace } from "./RoutingWorkspace";

describe("destination outcome", () => {
  it("explains safe recovery for an occupied output without hiding the error", () => {
    const html = renderToStaticMarkup(createElement(DestinationOutcome, {
      name: "Output A",
      item: {
        sourceId: "a", destinationId: "x", pipelineId: "p", state: "failed",
        assignmentId: null, assignmentGeneration: null,
        errorCode: "destination_in_use", errorMessage: "destination already has an assignment",
      },
    }));
    expect(html).toContain("destination already has an assignment");
    expect(html).toContain("If it is safe to disconnect");
    expect(html).toContain("wait for confirmation before retrying Take");
  });
});

describe("Salvo matrix display", () => {
  const sources: SourceRecord[] = ["a", "b"].map((id) => ({
    id, friendlyName: `Input ${id}`, sourceType: "alsa_l16",
    desiredState: "enabled", currentRevision: 1, revisionId: `${id}-1`,
    lifecycle: "validated", configuration: { kind: "alsa_l16", device: "hw:1,0" },
  }));
  const destinations: DestinationRecord[] = [{
    id: "x", friendlyName: "Output x", destinationType: "rtp_l16",
    desiredState: "enabled", currentRevision: 1, revisionId: "x-1",
    lifecycle: "validated",
    configuration: { kind: "rtp_l16", address: "127.0.0.1", port: 5004, payloadType: 96 },
  }];
  const routes: RouteRecord[] = [{
    sourceId: "b", destinationId: "x", desiredState: "desired",
    observedState: "running", observedSourceId: "a", revision: 3,
  }];
  const props = {
    sources, destinations, routes, routesLoading: false, routesError: "",
    selectedSourceId: "", selectedDestinationId: "", onSelect: () => {},
  };

  it("places enlarged output and input labels at opposite corners of the matrix header", () => {
    const html = renderToStaticMarkup(createElement(CrosspointMatrix, props));
    expect(html).toContain("OUTPUT →");
    expect(html).toContain("INPUT ↓");
    expect(html.indexOf("OUTPUT →")).toBeLessThan(html.indexOf("INPUT ↓"));
    expect(html).toContain("font-size:1.5rem");
    expect(html).toContain("top:8px;right:8px");
    expect(html).toContain("bottom:8px;left:8px");
  });

  it("starts blank despite existing desired and running routes", () => {
    const client = new QueryClient();
    client.setQueryData(["presets"], [{
      id: "saved-1", name: "Saved selection", revision: 1,
      routes: [{ sourceId: "b", destinationId: "x" }],
    }]);
    const html = renderToStaticMarkup(createElement(QueryClientProvider, { client },
      createElement(RoutingWorkspace, {
        ...props, mode: "salvos", apiBaseUrl: "", sourceError: "", destinationError: "",
      })));

    expect(html).toContain("Select Salvo");
    expect(html).not.toContain("Saved salvos");
    expect(html).not.toContain('aria-pressed="true"');
    expect(html).not.toContain("MuiButton-containedSuccess");
    expect(html).not.toContain("MuiButton-containedWarning");
    expect(html).not.toContain("saved desired routes");
    expect(html).toContain("Not selected for this salvo.");
    expect(html).toContain("Live routes are not shown.");
  });

  it("highlights only the saved selection without overlaying live routes", () => {
    const selections = snapshotPresetRoutes([{ sourceId: "b", destinationId: "x" }], routes);
    const html = renderToStaticMarkup(createElement(CrosspointMatrix, {
      ...props, selectionOnly: true, selections,
    }));

    expect(html).toContain('aria-label="Select Input b to Output x. Selected to activate this route." aria-pressed="true"');
    expect(html.match(/aria-pressed="true"/g)).toHaveLength(1);
    expect(html).not.toContain("MuiButton-containedSuccess");
    expect(html).not.toContain("Desired route revision");
  });

  it("renders a saved Unroute action red and identifies destination-wide disconnection", () => {
    const selections = snapshotPresetRoutes([{
      sourceId: "b", destinationId: "x", action: "stop",
    }], routes);
    const html = renderToStaticMarkup(createElement(CrosspointMatrix, {
      ...props, selectionOnly: true, selections,
    }));
    expect(html).toContain("MuiButton-containedError");
    expect(html).toContain("Selected to unroute this output, regardless of its input.");
    expect(html).not.toContain("MuiButton-containedPrimary");
    expect(html.match(/aria-pressed="true"/g)).toHaveLength(1);
  });

  it("preserves live indicators in the main routing grid", () => {
    const html = renderToStaticMarkup(createElement(CrosspointMatrix, props));

    expect(html).toContain("Worker-observed route is running.");
    expect(html).toContain("Desired route revision 3");
    expect(html).toContain("MuiButton-containedSuccess");
    expect(html).toContain("MuiButton-containedWarning");
    expect(html).toContain("1 saved desired routes.");
  });

  it("does not advertise Shift+F10 as a route action", () => {
    const client = new QueryClient();
    const html = renderToStaticMarkup(createElement(QueryClientProvider, { client },
      createElement(RoutingWorkspace, {
        ...props, mode: "routes", apiBaseUrl: "", sourceError: "", destinationError: "",
      })));

    expect(html).toContain("Right-click it for Take, Unroute, or Preview input.");
    expect(html).not.toContain("Shift+F10");
  });

  it("shows preview levels in the routing grid but not the salvo editor", () => {
    const client = new QueryClient();
    const preview = { name: "Input a", levels: { left: -12, right: -24 }, busy: true };
    const render = (mode: "routes" | "salvos") => renderToStaticMarkup(
      createElement(QueryClientProvider, { client }, createElement(RoutingWorkspace, {
        ...props, mode, apiBaseUrl: "", sourceError: "", destinationError: "", preview,
      })),
    );
    const routing = render("routes");
    expect(routing).toContain("Preview: Input a");
    expect(routing).toContain("Left -12.0 dBFS");
    expect(routing).toContain("Right -24.0 dBFS");
    expect(render("salvos")).not.toContain("Preview: Input a");
  });
});

describe("getVisibleMatrixRange", () => {
  it("limits rendering to visible items plus overscan", () => {
    expect(getVisibleMatrixRange(200, 80, 64, 40, 1)).toEqual({
      start: 4,
      end: 8,
    });
  });

  describe("getDesiredOutputCount", () => {
    it("counts only desired routes for the requested input", () => {
      const routes: RouteRecord[] = [
        {
          destinationId: "destination-1",
          sourceId: "source-a",
          desiredState: "desired",
          observedState: "running",
          revision: 1,
        },
        {
          destinationId: "destination-2",
          sourceId: "source-a",
          desiredState: "desired",
          observedState: "preparing",
          revision: 1,
        },
        {
          destinationId: "destination-3",
          sourceId: "source-b",
          desiredState: "desired",
          observedState: "running",
          revision: 1,
        },
        {
          destinationId: "destination-4",
          sourceId: "source-a",
          desiredState: "removed",
          observedState: "stopped",
          revision: 2,
        },
      ];

      expect(getDesiredOutputCount(routes, "source-a")).toBe(2);
      expect(getDesiredOutputCount(routes, "source-b")).toBe(1);
      expect(getDesiredOutputCount(routes, "source-c")).toBe(0);
    });
  });

  it("clamps the range at both ends of the matrix", () => {
    expect(getVisibleMatrixRange(-20, 40, 3, 40, 1)).toEqual({
      start: 0,
      end: 2,
    });
    expect(getVisibleMatrixRange(1000, 100, 3, 40, 1)).toEqual({
      start: 3,
      end: 3,
    });
  });

  describe("isObservedRunningRoute", () => {
    it("keeps the previous observed source visible when desired source changes", () => {
      const route = {
        destinationId: "destination-1",
        sourceId: "source-b",
        desiredState: "desired",
        observedState: "running",
        observedSourceId: "source-a",
        revision: 2,
      };

      expect(isObservedRunningRoute(route, "source-a")).toBe(true);
      expect(isObservedRunningRoute(route, "source-b")).toBe(false);
    });

    it("does not mark an unobserved desired source as active", () => {
      const route = {
        destinationId: "destination-1",
        sourceId: "source-b",
        desiredState: "desired",
        observedState: "unknown",
        observedSourceId: null,
        revision: 2,
      };

      expect(isObservedRunningRoute(route, "source-b")).toBe(false);
    });
  });
});
