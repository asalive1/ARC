import { describe, expect, it } from "vitest";

import {
  hasStaleRouteSelection, matchesPresetRoutes, snapshotPresetRoutes,
  stageRouteSelection, toggleRouteSelection,
} from "./routeStaging";

describe("salvo selections", () => {
  const first = { sourceId: "a", destinationId: "x", expectedRouteRevision: 3 };
  const second = { sourceId: "b", destinationId: "y", expectedRouteRevision: 0 };

  it("toggles one destination off without losing sibling selections", () => {
    expect(toggleRouteSelection([first, second], first)).toEqual([second]);
    expect(toggleRouteSelection([first, second], { ...first, sourceId: "c" }))
      .toEqual([{ ...first, sourceId: "c" }, second]);
  });

  it("pins route revisions at load and detects subsequent destination changes", () => {
    const saved = [{ sourceId: "a", destinationId: "x" }, { sourceId: "b", destinationId: "y" }];
    const routes = [{
      sourceId: "c", destinationId: "x", revision: 3,
      desiredState: "desired", observedState: "running",
    }];
    const snapshot = snapshotPresetRoutes(saved, routes);
    expect(snapshot).toEqual([first, second]);
    expect(hasStaleRouteSelection(snapshot, routes)).toBe(false);
    expect(hasStaleRouteSelection(snapshot, [{ ...routes[0], revision: 4 }])).toBe(true);
    expect(snapshot[0].expectedRouteRevision).toBe(3);
  });

  it("compares saved route sets independent of selection order", () => {
    expect(matchesPresetRoutes([second, first], [first, second])).toBe(true);
    expect(matchesPresetRoutes([first], [first, second])).toBe(false);
    expect(matchesPresetRoutes([{ ...first, sourceId: "c" }, second], [first, second])).toBe(false);
  });
});

describe("stageRouteSelection", () => {
  it("adds a staged route for a new destination", () => {
    expect(
      stageRouteSelection(
        [{
          sourceId: "source-a",
          destinationId: "destination-a",
          expectedRouteRevision: 0,
        }],
        {
          sourceId: "source-b",
          destinationId: "destination-b",
          expectedRouteRevision: 0,
        },
      ),
    ).toEqual([
      {
        sourceId: "source-a",
        destinationId: "destination-a",
        expectedRouteRevision: 0,
      },
      {
        sourceId: "source-b",
        destinationId: "destination-b",
        expectedRouteRevision: 0,
      },
    ]);
  });

  it("does not duplicate the same source and destination when the revision matches", () => {
    const staged = [{
      sourceId: "source-a",
      destinationId: "destination-a",
      expectedRouteRevision: 2,
    }];

    expect(
      stageRouteSelection(staged, {
        sourceId: "source-a",
        destinationId: "destination-a",
        expectedRouteRevision: 2,
      }),
    ).toBe(staged);
  });

  it("refreshes the pinned route revision when the same selection is restaged", () => {
    expect(
      stageRouteSelection(
        [{
          sourceId: "source-a",
          destinationId: "destination-a",
          expectedRouteRevision: 2,
        }],
        {
          sourceId: "source-a",
          destinationId: "destination-a",
          expectedRouteRevision: 3,
        },
      ),
    ).toEqual([{
      sourceId: "source-a",
      destinationId: "destination-a",
      expectedRouteRevision: 3,
    }]);
  });

  it("replaces a staged source for the same destination", () => {
    expect(
      stageRouteSelection(
        [
          {
            sourceId: "source-a",
            destinationId: "destination-a",
            expectedRouteRevision: 0,
          },
          {
            sourceId: "source-b",
            destinationId: "destination-b",
            expectedRouteRevision: 4,
          },
        ],
        {
          sourceId: "source-c",
          destinationId: "destination-a",
          expectedRouteRevision: 2,
        },
      ),
    ).toEqual([
      {
        sourceId: "source-c",
        destinationId: "destination-a",
        expectedRouteRevision: 2,
      },
      {
        sourceId: "source-b",
        destinationId: "destination-b",
        expectedRouteRevision: 4,
      },
    ]);
  });
});
