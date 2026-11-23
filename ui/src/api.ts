export interface Liveness {
  status: "alive";
}

export async function fetchLiveness(
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<Liveness> {
  const response = await fetcher(`${baseUrl}/api/v1/health/live`);
  if (!response.ok) {
    throw new Error(`API health request failed with HTTP ${response.status}`);
  }

  const body: unknown = await response.json();
  if (
    typeof body !== "object" ||
    body === null ||
    !("status" in body) ||
    body.status !== "alive"
  ) {
    throw new Error("API returned an invalid liveness response");
  }

  return { status: "alive" };
}
