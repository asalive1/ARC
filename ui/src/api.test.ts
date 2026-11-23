import { describe, expect, it, vi } from "vitest";

import {
  createDestination,
  createSource,
  fetchLiveness,
  fetchSources,
  planRoute,
} from "./api";

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

  it("surfaces API errors for catalog operations", async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response("invalid request", { status: 422 }),
    );

    await expect(
      createSource("Bad source", { kind: "alsa_l16", device: "hw:1,0" }, "", fetcher),
    ).rejects.toThrow("API request failed with HTTP 422");
  });
});
