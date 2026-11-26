import { describe, expect, it } from "vitest";

import {
  getDesiredOutputCount,
  getVisibleMatrixRange,
  isObservedRunningRoute,
} from "./CrosspointMatrix";
import type { RouteRecord } from "./api";

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
