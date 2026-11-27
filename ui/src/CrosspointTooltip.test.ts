import { createElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

vi.mock("@mui/material", async (importOriginal) => {
  const original = await importOriginal<typeof import("@mui/material")>();
  return {
    ...original,
    Tooltip: ({ title, children }: { title: string; children: ReactNode }) =>
      createElement("span", { "data-tooltip": title }, children),
  };
});

import { CrosspointMatrix } from "./CrosspointMatrix";
import type { DestinationRecord, RouteRecord, SourceRecord } from "./api";

describe("crosspoint tooltip", () => {
  it("shows only input and output names while retaining the accessible route description", () => {
    const sources: SourceRecord[] = [{
      id: "a", friendlyName: "Input A", sourceType: "alsa_l16",
      desiredState: "enabled", currentRevision: 1, revisionId: "a-1",
      lifecycle: "validated", configuration: { kind: "alsa_l16", device: "hw:1,0" },
    }];
    const destinations: DestinationRecord[] = [{
      id: "b", friendlyName: "Output B", destinationType: "rtp_l16",
      desiredState: "enabled", currentRevision: 1, revisionId: "b-1",
      lifecycle: "validated",
      configuration: { kind: "rtp_l16", address: "127.0.0.1", port: 5004, payloadType: 96 },
    }];
    const routes: RouteRecord[] = [{
      sourceId: "a", destinationId: "b", desiredState: "desired",
      observedState: "running", observedSourceId: "a", revision: 3,
    }];
    const html = renderToStaticMarkup(createElement(CrosspointMatrix, {
      sources, destinations, routes, routesLoading: false, routesError: "",
      selectedSourceId: "", selectedDestinationId: "", onSelect: () => {},
    }));

    expect(html).toContain('data-tooltip="Input A:Output B"');
    expect(html).toContain('aria-label="Select Input A to Output B. Desired route.');
    expect(html).toContain("Worker-observed route is running.");
  });
});
