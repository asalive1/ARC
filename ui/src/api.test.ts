import { describe, expect, it, vi } from "vitest";

import {
  ApiRequestError,
  cancelAssignment,
  cancelCapacityRequest,
  createDestination,
  createSource,
  deleteSource,
  deleteDestination,
  fetchPipelineAssignment,
  fetchCapacityRequest,
  fetchAdminAuthStatus,
  fetchLiveness,
  fetchReadiness,
  fetchRoutes,
  fetchPresets,
  savePreset,
  deletePreset,
  executePreset,
  unrouteDestination,
  fetchSources,
  getRouteBatchOperation,
  planRoute,
  planRouteAssignment,
  setDesiredRoute,
  setDesiredRoutesBatch,
  startAssignment,
  stopAssignment,
  updateDestination,
  updateSource,
  validateDestination,
  validateSource,
} from "./api";

describe("API error details", () => {
  const route = {
    sourceId: "source-1", destinationId: "destination-1",
    expectedRouteRevision: 0, confirmReplace: false,
  };

  it("preserves the server reason, code and status for a rejected Take", async () => {
    const message = "media conversion is not supported by Phase 1";
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(JSON.stringify({ code: "invalid_request", message }), { status: 422 }),
    );

    await expect(setDesiredRoutesBatch([route], "take-1", "", fetcher)).rejects.toMatchObject({
      name: "ApiRequestError",
      status: 422,
      code: "invalid_request",
      message: `API request failed with HTTP 422: ${message}`,
    });
  });

  it("uses the same error details for authentication requests", async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(JSON.stringify({ code: "unauthorized", message: "Sign in first" }), { status: 401 }),
    );

    await expect(fetchAdminAuthStatus("", fetcher)).rejects.toMatchObject({
      status: 401, code: "unauthorized", message: "API request failed with HTTP 401: Sign in first",
    });
  });

  it("preserves HTTP status when the error response body cannot be read", async () => {
    const body = new ReadableStream({
      start(controller) { controller.error(new TypeError("Connection closed")); },
    });
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(new Response(body, { status: 422 }));

    await expect(setDesiredRoutesBatch([route], "take-1", "", fetcher)).rejects.toMatchObject({
      status: 422,
      message: "API request failed with HTTP 422: The API error response could not be read: Connection closed",
    });
  });

  it.each([
    ["malformed JSON", "{", "application/json"],
    ["an invalid envelope", '{"code":42,"message":null}', "application/json"],
    ["an HTML proxy error", "<html>Bad gateway</html>", "text/html"],
  ])("retains a typed HTTP error for %s", async (_label, body, contentType) => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(body, { status: 502, headers: { "content-type": contentType } }),
    );
    const request = setDesiredRoutesBatch([route], "take-1", "", fetcher);

    await expect(request).rejects.toBeInstanceOf(ApiRequestError);
    await expect(request).rejects.toMatchObject({
      status: 502,
      message: "API request failed with HTTP 502: The API returned an invalid error response",
    });
  });
});

describe("catalog deletion API", () => {
  it.each([
    ["sources", deleteSource], ["destinations", deleteDestination],
  ] as const)("revision-guards %s deletion and requires confirmation", async (path, remove) => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(JSON.stringify({ deleted: true })),
    );
    await remove("resource/id", 3, "", fetcher);
    expect(fetcher).toHaveBeenCalledWith(`/api/v1/${path}/resource%2Fid`, expect.objectContaining({
      method: "DELETE", body: JSON.stringify({ expectedRevision: 3 }),
    }));
    fetcher.mockResolvedValue(new Response("{}"));
    await expect(remove("resource", 3, "", fetcher)).rejects.toThrow("did not confirm");
    fetcher.mockResolvedValue(new Response("null"));
    await expect(remove("resource", 3, "", fetcher)).rejects.toThrow("did not confirm");
    fetcher.mockResolvedValue(new Response("conflict", { status: 409 }));
    await expect(remove("resource", 3, "", fetcher)).rejects.toThrow("HTTP 409");
  });
});

describe("salvo and unroute API", () => {
  const routes = [{ sourceId: "input-a", destinationId: "output-a" }];
  const preset = { id: "salvo-a", name: "Morning", revision: 2, routes };
  const operation = { operationId: "operation-a", state: "pending", routes: [], results: [] };
  const respond = (value: unknown) =>
    vi.fn<typeof fetch>().mockResolvedValue(new Response(JSON.stringify(value)));

  it("loads saved salvos and rejects malformed persisted data", async () => {
    await expect(fetchPresets("", respond([preset]))).resolves.toEqual([preset]);
    await expect(fetchPresets("", respond([{ ...preset, routes: [{}] }])))
      .rejects.toThrow("invalid salvo collection");
  });

  it("round-trips saved Unroute actions and rejects unknown action kinds", async () => {
    const mixed = { ...preset, routes: [{ ...routes[0], action: "stop" as const }] };
    await expect(fetchPresets("", respond([mixed]))).resolves.toEqual([mixed]);
    const fetcher = respond(mixed);
    await savePreset({ name: mixed.name, routes: mixed.routes }, "", fetcher);
    expect(fetcher).toHaveBeenCalledWith("/api/v1/presets", expect.objectContaining({
      body: JSON.stringify({ name: mixed.name, routes: mixed.routes }),
    }));
    const input = {
      expectedRevision: 2,
      routes: [{ ...mixed.routes[0], expectedRouteRevision: 7, confirmReplace: false }],
    };
    const execute = respond(operation);
    await executePreset(preset.id, input, "mixed-key", "", execute);
    expect(execute).toHaveBeenCalledWith("/api/v1/presets/salvo-a/execute",
      expect.objectContaining({ body: JSON.stringify(input) }));
    await expect(fetchPresets("", respond([{ ...preset, routes: [{ ...routes[0], action: "erase" }] }])))
      .rejects.toThrow("invalid salvo collection");
  });

  it("creates and revision-guards updates without embedding route-state revisions", async () => {
    const create = respond(preset);
    await savePreset({ name: "Morning", routes }, "", create);
    expect(create).toHaveBeenCalledWith("/api/v1/presets", expect.objectContaining({
      method: "POST", body: JSON.stringify({ name: "Morning", routes }),
    }));
    const update = respond(preset);
    await savePreset({ id: "salvo-a", name: "Morning", routes, expectedRevision: 1 }, "", update);
    expect(update).toHaveBeenCalledWith("/api/v1/presets/salvo-a", expect.objectContaining({
      method: "PUT", body: JSON.stringify({ name: "Morning", routes, expectedRevision: 1 }),
    }));
  });

  it("takes a saved salvo using the preview revisions and operation key", async () => {
    const fetcher = respond(operation);
    const input = {
      expectedRevision: 2,
      routes: [{ ...routes[0], expectedRouteRevision: 7, confirmReplace: true }],
    };
    await expect(executePreset("salvo-a", input, "retry-key", "", fetcher)).resolves.toEqual(operation);
    expect(fetcher).toHaveBeenCalledWith("/api/v1/presets/salvo-a/execute", expect.objectContaining({
      method: "POST", headers: expect.objectContaining({ "idempotency-key": "retry-key" }),
      body: JSON.stringify(input),
    }));
  });

  it("unroutes a destination with a revision guard rather than a session assignment", async () => {
    const fetcher = respond(operation);
    const input = { destinationId: "output-a", expectedRouteRevision: 7 };
    await expect(unrouteDestination(input, "unroute-key", "", fetcher)).resolves.toEqual(operation);
    expect(fetcher).toHaveBeenCalledWith("/api/v1/routes/unroute", expect.objectContaining({
      method: "POST", body: JSON.stringify(input),
      headers: expect.objectContaining({ "idempotency-key": "unroute-key" }),
    }));
    const conflict = vi.fn<typeof fetch>().mockResolvedValue(new Response("conflict", { status: 409 }));
    await expect(unrouteDestination(input, "unroute-key", "", conflict)).rejects.toThrow("HTTP 409");
  });

  it("revision-guards deletion and requires an explicit confirmation response", async () => {
    const fetcher = respond({ deleted: true });
    await deletePreset("salvo-a", 2, "", fetcher);
    expect(fetcher).toHaveBeenCalledWith("/api/v1/presets/salvo-a", expect.objectContaining({
      method: "DELETE", body: JSON.stringify({ expectedRevision: 2 }),
    }));
    await expect(deletePreset("salvo-a", 2, "", respond({}))).rejects.toThrow("did not confirm");
  });
});

describe("fetchLiveness", () => {
  it("requests the versioned API liveness endpoint", async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(JSON.stringify({ status: "alive" }), { status: 200 }),
    );

    await expect(fetchLiveness("http://arc-api:8080", fetcher)).resolves.toEqual({
      status: "alive",
    });

    expect(fetcher).toHaveBeenCalledWith(
      "http://arc-api:8080/api/v1/health/live",
    );
  });

  it("surfaces API errors instead of reporting a healthy fallback", async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response("unavailable", { status: 503 }),
    );

    await expect(fetchLiveness("", fetcher)).rejects.toThrow(
      "API health request failed with HTTP 503",
    );
  });

  it("rejects malformed health responses", async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(JSON.stringify({ status: "unknown" }), { status: 200 }),
    );

    await expect(fetchLiveness("", fetcher)).rejects.toThrow(
      "API returned an invalid liveness response",
    );
  });
});

describe("fetchReadiness", () => {
  it("requests the database readiness endpoint", async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(JSON.stringify({ status: "ready", database: "ready" }), {
        status: 200,
      }),
    );

    await expect(fetchReadiness("http://arc-api:8080", fetcher)).resolves.toEqual({
      status: "ready",
      database: "ready",
    });
    expect(fetcher).toHaveBeenCalledWith(
      "http://arc-api:8080/api/v1/health/ready",
    );
  });

  it("surfaces database readiness failures", async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(
        JSON.stringify({ status: "not_ready", database: "unavailable" }),
        { status: 503 },
      ),
    );

    await expect(fetchReadiness("", fetcher)).rejects.toThrow(
      "API readiness request failed with HTTP 503",
    );
  });
});

describe("catalog and route API", () => {
  it("creates a source using the camelCase media profile contract", async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(JSON.stringify({ id: "source-1" }), { status: 201 }),
    );

    await createSource(
      "RTP input",
      { kind: "rtp_mp2t", bindAddress: "0.0.0.0", port: 5000 },
      "http://arc-api:8080",
      fetcher,
    );

    expect(fetcher).toHaveBeenCalledWith(
      "http://arc-api:8080/api/v1/sources",
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          friendlyName: "RTP input",
          profile: {
            kind: "rtp_mp2t",
            bindAddress: "0.0.0.0",
            port: 5000,
          },
        }),
      },
    );
  });

  it("reads source collections", async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(JSON.stringify([]), { status: 200 }),
    );

    await expect(fetchSources("", fetcher)).resolves.toEqual([]);
    expect(fetcher).toHaveBeenCalledWith("/api/v1/sources", undefined);
  });

  it("reads and validates the current route collection", async () => {
    const route = {
      destinationId: "destination-1",
      sourceId: "source-1",
      desiredState: "desired",
      observedState: "running",
      observedSourceId: "source-1",
      revision: 3,
    };
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(JSON.stringify({ items: [route] }), { status: 200 }),
    );

    await expect(fetchRoutes("", fetcher)).resolves.toEqual([route]);
    expect(fetcher).toHaveBeenCalledWith("/api/v1/routes", undefined);
  });

  it("rejects malformed route collection entries", async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(JSON.stringify({ items: [{ destinationId: "destination-1" }] }), {
        status: 200,
      }),
    );

    await expect(fetchRoutes("", fetcher)).rejects.toThrow(
      "API returned an invalid route collection",
    );
  });

  it("writes desired route state with revision and replacement confirmation", async () => {
    const input = {
      sourceId: "source-2",
      destinationId: "destination-1",
      expectedRouteRevision: 3,
      confirmReplace: true,
    };
    const route = {
      destinationId: input.destinationId,
      sourceId: input.sourceId,
      desiredState: "desired",
      observedState: "unknown",
      observedSourceId: null,
      revision: 4,
    };
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(JSON.stringify(route), { status: 200 }),
    );

    await expect(setDesiredRoute(input, "", fetcher)).resolves.toEqual(route);
    expect(fetcher).toHaveBeenCalledWith(
      "/api/v1/routes",
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(input),
      },
    );
  });

  it("writes an idempotent desired-route batch", async () => {
    const operation = {
      operationId: "operation-1",
      state: "pending",
      routes: [{
        destinationId: "destination-1",
        sourceId: "source-1",
        desiredState: "desired",
        observedState: "unknown",
        observedSourceId: null,
        revision: 1,
      }],
      results: [{
        sourceId: "source-1",
        destinationId: "destination-1",
        pipelineId: "pipeline-1",
        state: "pending",
        assignmentId: null,
        assignmentGeneration: null,
        errorCode: null,
        errorMessage: null,
      }],
    };
    const routes = [{
      sourceId: "source-1",
      destinationId: "destination-1",
      expectedRouteRevision: 0,
      confirmReplace: false,
    }];
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(JSON.stringify(operation), { status: 200 }),
    );

    await expect(
      setDesiredRoutesBatch(routes, "idempotency-key", "", fetcher),
    ).resolves.toEqual(operation);
    expect(fetcher).toHaveBeenCalledWith(
      "/api/v1/routes/batch",
      {
        method: "POST",
        headers: {
          "content-type": "application/json",
          "idempotency-key": "idempotency-key",
        },
        body: JSON.stringify({ routes }),
      },
    );
  });

  it("retrieves a route batch operation with per-route outcomes", async () => {
    const operation = {
      operationId: "operation-1",
      state: "partial",
      routes: [],
      results: [{
        sourceId: "source-1",
        destinationId: "destination-1",
        pipelineId: "pipeline-1",
        state: "failed",
        assignmentId: null,
        assignmentGeneration: null,
        errorCode: "placement_unavailable",
        errorMessage: "No eligible worker was available",
      }],
    };
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(JSON.stringify(operation), { status: 200 }),
    );

    await expect(
      getRouteBatchOperation("operation-1", "", fetcher),
    ).resolves.toEqual(operation);
    expect(fetcher).toHaveBeenCalledWith(
      "/api/v1/operations/routes/operation-1",
      {},
    );
  });

  it("updates a source by creating a new draft revision", async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(JSON.stringify({ id: "source-1" }), { status: 200 }),
    );

    await updateSource(
      "source/1",
      "Edited input",
      { kind: "rtp_l24", bindAddress: "0.0.0.0", port: 5004, payloadType: 96 },
      "http://arc-api:8080",
      fetcher,
    );

    expect(fetcher).toHaveBeenCalledWith(
      "http://arc-api:8080/api/v1/sources/source%2F1",
      {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          friendlyName: "Edited input",
          profile: {
            kind: "rtp_l24",
            bindAddress: "0.0.0.0",
            port: 5004,
            payloadType: 96,
          },
        }),
      },
    );
  });

  it("creates destinations and plans routes", async () => {
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(new Response(JSON.stringify({ id: "dest-1" }), { status: 201 }))
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ mediaPath: "rtp_mp2t_to_srt_mpeg_ts" }), {
          status: 200,
        }),
      );

    await createDestination(
      "SRT output",
      { kind: "srt_mpeg_ts", host: "127.0.0.1", port: 6000, mode: "caller" },
      "",
      fetcher,
    );
    await planRoute(
      { kind: "rtp_mp2t", bindAddress: "0.0.0.0", port: 5000 },
      { kind: "srt_mpeg_ts", host: "127.0.0.1", port: 6000, mode: "caller" },
      "",
      fetcher,
    );

    expect(fetcher).toHaveBeenNthCalledWith(
      1,
      "/api/v1/destinations",
      expect.objectContaining({ method: "POST" }),
    );
    expect(fetcher).toHaveBeenNthCalledWith(
      2,
      "/api/v1/routes/plan",
      expect.objectContaining({ method: "POST" }),
    );
  });

  it("updates a destination", async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(JSON.stringify({ id: "destination-1" }), { status: 200 }),
    );

    await updateDestination(
      "destination-1",
      "Edited output",
      { kind: "srt_mpeg_ts", host: "192.0.2.10", port: 6002, mode: "listener" },
      "",
      fetcher,
    );

    expect(fetcher).toHaveBeenCalledWith(
      "/api/v1/destinations/destination-1",
      {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          friendlyName: "Edited output",
          profile: {
            kind: "srt_mpeg_ts",
            host: "192.0.2.10",
            port: 6002,
            mode: "listener",
          },
        }),
      },
    );
  });

  it("plans a matching RTP L24 path", async () => {
    const result = {
      mediaPath: "rtp_l24_to_rtp_l24",
      estimate: {
        capacityUnits: 2,
        ingressPayloadBitsPerSecond: null,
        egressPayloadBitsPerSecond: null,
      },
    };
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(JSON.stringify(result), {
        status: 200,
      }),
    );

    await expect(
      planRoute(
        {
          kind: "rtp_l24",
          bindAddress: "0.0.0.0",
          port: 5004,
          payloadType: 96,
        },
        {
          kind: "rtp_l24",
          address: "127.0.0.1",
          port: 5006,
          payloadType: 97,
        },
        "",
        fetcher,
      ),
    ).resolves.toEqual(result);
    expect(fetcher).toHaveBeenCalledWith(
      "/api/v1/routes/plan",
      expect.objectContaining({
        body: JSON.stringify({
          source: {
            kind: "rtp_l24",
            bindAddress: "0.0.0.0",
            port: 5004,
            payloadType: 96,
          },
          destination: {
            kind: "rtp_l24",
            address: "127.0.0.1",
            port: 5006,
            payloadType: 97,
          },
        }),
      }),
    );
  });

  it("supports older route plans but rejects malformed estimates", async () => {
    const source = { kind: "alsa_l16" as const, device: "hw:1,0" };
    const destination = {
      kind: "rtp_l16" as const,
      address: "127.0.0.1",
      port: 5004,
      payloadType: 96,
    };
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ mediaPath: "alsa_l16_to_rtp_l16" })),
      )
      .mockResolvedValueOnce(
        new Response(JSON.stringify({
          mediaPath: "alsa_l16_to_rtp_l16",
          estimate: { capacityUnits: 2, ingressPayloadBitsPerSecond: null },
        })),
      )
      .mockResolvedValueOnce(
        new Response(JSON.stringify({
          mediaPath: "alsa_l16_to_rtp_l16",
          estimate: {
            capacityUnits: 2,
            ingressPayloadBitsPerSecond: null,
            egressPayloadBitsPerSecond: 1536000,
          },
        })),
      );
    await expect(planRoute(source, destination, "", fetcher)).resolves.toEqual({
      mediaPath: "alsa_l16_to_rtp_l16",
    });
    await expect(planRoute(source, destination, "", fetcher)).rejects.toThrow(
      "API returned an invalid route estimate",
    );
    await expect(planRoute(source, destination, "", fetcher)).resolves.toEqual({
      mediaPath: "alsa_l16_to_rtp_l16",
      estimate: {
        capacityUnits: 2,
        ingressPayloadBitsPerSecond: null,
        egressPayloadBitsPerSecond: 1536000,
      },
    });
  });

  describe("assignment API", () => {
    it("opts into waiting and reads or cancels the returned request", async () => {
      const waiting = {
        state: "waiting", requestId: "request/1", expiresAt: "2026-01-01T00:05:00Z",
        mediaPath: "rtp_l16_to_rtp_l16",
        estimate: { capacityUnits: 2, ingressPayloadBitsPerSecond: null, egressPayloadBitsPerSecond: null },
        assignment: null, rejections: [],
      };
      const fetcher = vi.fn<typeof fetch>().mockImplementation(async () =>
        new Response(JSON.stringify(waiting), { status: 200 }),
      );
      await planRouteAssignment({
        pipelineId: "pipeline", idempotencyKey: "key", sourceId: "source",
        sourceRevision: 1, destinationId: "destination", destinationRevision: 1,
        allowWaiting: true, capacityRequestId: "local-only",
      }, "", fetcher);
      expect(JSON.parse((fetcher.mock.calls[0][1] as RequestInit).body as string)).toEqual({
        pipelineId: "pipeline", sourceId: "source", sourceRevision: 1,
        destinationId: "destination", destinationRevision: 1, allowWaiting: true,
      });
      await expect(fetchCapacityRequest("request/1", "", fetcher)).resolves.toEqual(waiting);
      await cancelCapacityRequest("request/1", "", fetcher);
      expect(fetcher.mock.calls[1][0]).toBe("/api/v1/capacity-requests/request%2F1");
      expect(fetcher.mock.calls[2]).toEqual([
        "/api/v1/capacity-requests/request%2F1/cancel", { method: "POST" },
      ]);
    });
    it("plans a capacity-reserving assignment with pinned endpoint revisions", async () => {
      const planned = {
        state: "planned",
        mediaPath: "rtp_mp2t_to_srt_mpeg_ts",
        estimate: {
          capacityUnits: 2,
          ingressPayloadBitsPerSecond: null,
          egressPayloadBitsPerSecond: null,
        },
        assignment: {
          id: "assignment-1",
          pipelineId: "pipeline-1",
          workerId: "worker-1",
          generation: 1,
        },
        rejections: [],
      };
      const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
        new Response(JSON.stringify(planned), { status: 200 }),
      );

      await expect(
        planRouteAssignment(
          {
            pipelineId: "pipeline-1",
            idempotencyKey: "6e474805-5c0c-4d7b-b4a2-985c15f825b7",
            sourceId: "source-1",
            sourceRevision: 3,
            destinationId: "destination-1",
            destinationRevision: 4,
          },
          "http://arc-api:8080",
          fetcher,
        ),
      ).resolves.toEqual(planned);
      expect(fetcher).toHaveBeenCalledWith(
        "http://arc-api:8080/api/v1/routes/plan-assignment",
        {
          method: "POST",
          headers: {
            "content-type": "application/json",
            "Idempotency-Key": "6e474805-5c0c-4d7b-b4a2-985c15f825b7",
          },
          body: JSON.stringify({
            pipelineId: "pipeline-1",
            sourceId: "source-1",
            sourceRevision: 3,
            destinationId: "destination-1",
            destinationRevision: 4,
          }),
        },
      );
    });

    it("looks up an assignment and sends generation-fenced lifecycle requests", async () => {
      const snapshot = {
        assignment: {
          id: "assignment-1",
          pipelineId: "pipeline/1",
          workerId: "worker-1",
          generation: 7,
        },
        desiredState: "running",
        observedState: "running",
      };
      const fetcher = vi
        .fn<typeof fetch>()
        .mockResolvedValueOnce(
          new Response(JSON.stringify(snapshot), { status: 200 }),
        )
        .mockResolvedValueOnce(
          new Response(JSON.stringify(snapshot), { status: 200 }),
        )
        .mockResolvedValueOnce(
          new Response(JSON.stringify(snapshot), { status: 200 }),
        )
        .mockResolvedValueOnce(
          new Response(
            JSON.stringify({
              assignmentId: "assignment/1",
              generation: 7,
              state: "cancelled",
            }),
            { status: 200 },
          ),
        );

      await expect(
        fetchPipelineAssignment("pipeline/1", "", fetcher),
      ).resolves.toEqual(snapshot);
      await startAssignment("assignment/1", 7, "", fetcher);
      await stopAssignment("assignment/1", 7, "", fetcher);
      await expect(
        cancelAssignment("assignment/1", 7, "", fetcher),
      ).resolves.toEqual({
        assignmentId: "assignment/1",
        generation: 7,
        state: "cancelled",
      });

      expect(fetcher).toHaveBeenNthCalledWith(
        1,
        "/api/v1/pipelines/pipeline%2F1/assignment",
        undefined,
      );
      expect(fetcher).toHaveBeenNthCalledWith(
        2,
        "/api/v1/assignments/assignment%2F1/start",
        {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ generation: 7 }),
        },
      );
      expect(fetcher).toHaveBeenNthCalledWith(
        3,
        "/api/v1/assignments/assignment%2F1/stop",
        {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ generation: 7 }),
        },
      );
      expect(fetcher).toHaveBeenNthCalledWith(
        4,
        "/api/v1/assignments/assignment%2F1/cancel",
        {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ generation: 7 }),
        },
      );
    });

    it("preserves HTTP status for assignment lookup errors", async () => {
      const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
        new Response("not found", { status: 404 }),
      );

      await expect(
        fetchPipelineAssignment("missing-pipeline", "", fetcher),
      ).rejects.toMatchObject({ status: 404 });
    });
  });

  it("surfaces API errors for catalog operations", async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response("invalid request", { status: 422 }),
    );

    await expect(
      createSource("Bad source", { kind: "alsa_l16", device: "hw:1,0" }, "", fetcher),
    ).rejects.toThrow("API request failed with HTTP 422: invalid request");
  });

  it("validates source and destination drafts through their lifecycle endpoints", async () => {
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(new Response(JSON.stringify({ id: "source-1" }), { status: 200 }))
      .mockResolvedValueOnce(new Response(JSON.stringify({ id: "dest-1" }), { status: 200 }));

    await validateSource("source/1", "http://arc-api:8080", fetcher);
    await validateDestination("dest-1", "", fetcher);

    expect(fetcher).toHaveBeenNthCalledWith(
      1,
      "http://arc-api:8080/api/v1/sources/source%2F1/validate",
      { method: "POST" },
    );
    expect(fetcher).toHaveBeenNthCalledWith(
      2,
      "/api/v1/destinations/dest-1/validate",
      { method: "POST" },
    );
  });
});
