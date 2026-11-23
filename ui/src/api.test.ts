import { describe, expect, it, vi } from "vitest";

import { fetchLiveness } from "./api";

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
