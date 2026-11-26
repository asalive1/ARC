import { describe, expect, it } from "vitest";

import { stageRouteSelection } from "./routeStaging";

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
