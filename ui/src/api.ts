export interface Liveness {
  status: "alive";
}

export type SourceMediaProfile =
  | { kind: "alsa_l16"; device: string }
  | { kind: "rtp_mp2t"; bindAddress: string; port: number }
  | {
      kind: "rtp_l24";
      bindAddress: string;
      port: number;
      payloadType: number;
    };

export type DestinationMediaProfile =
  | { kind: "rtp_l16"; address: string; port: number; payloadType: number }
  | { kind: "rtp_l24"; address: string; port: number; payloadType: number }
  | {
      kind: "srt_mpeg_ts";
      host: string;
      port: number;
      mode: "caller" | "listener";
    };

interface CatalogRecord<Profile> {
  id: string;
  friendlyName: string;
  desiredState: "disabled" | "enabled";
  currentRevision: number;
  revisionId: string;
  lifecycle: "draft" | "validated" | "active" | "retired";
  configuration: Profile;
}

export interface SourceRecord extends CatalogRecord<SourceMediaProfile> {
  sourceType: string;
}

export interface DestinationRecord
  extends CatalogRecord<DestinationMediaProfile> {
  destinationType: string;
}

export interface RoutePlan {
  mediaPath:
    | "alsa_l16_to_rtp_l16"
    | "rtp_l24_to_rtp_l24"
    | "rtp_mp2t_to_srt_mpeg_ts";
}

async function requestJson<T>(
  path: string,
  init: RequestInit | undefined,
  baseUrl: string,
  fetcher: typeof fetch,
): Promise<T> {
  const response = await fetcher(`${baseUrl}/api/v1${path}`, init);
  if (!response.ok) {
    throw new Error(`API request failed with HTTP ${response.status}`);
  }
  return (await response.json()) as T;
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

export function fetchSources(
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<SourceRecord[]> {
  return requestJson("/sources", undefined, baseUrl, fetcher);
}

export function createSource(
  friendlyName: string,
  profile: SourceMediaProfile,
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<SourceRecord> {
  return requestJson(
    "/sources",
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ friendlyName, profile }),
    },
    baseUrl,
    fetcher,
  );
}

export function validateSource(
  sourceId: string,
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<SourceRecord> {
  return requestJson(
    `/sources/${encodeURIComponent(sourceId)}/validate`,
    { method: "POST" },
    baseUrl,
    fetcher,
  );
}

export function fetchDestinations(
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<DestinationRecord[]> {
  return requestJson("/destinations", undefined, baseUrl, fetcher);
}

export function createDestination(
  friendlyName: string,
  profile: DestinationMediaProfile,
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<DestinationRecord> {
  return requestJson(
    "/destinations",
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ friendlyName, profile }),
    },
    baseUrl,
    fetcher,
  );
}

export function validateDestination(
  destinationId: string,
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<DestinationRecord> {
  return requestJson(
    `/destinations/${encodeURIComponent(destinationId)}/validate`,
    { method: "POST" },
    baseUrl,
    fetcher,
  );
}

export function planRoute(
  source: SourceMediaProfile,
  destination: DestinationMediaProfile,
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<RoutePlan> {
  return requestJson(
    "/routes/plan",
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ source, destination }),
    },
    baseUrl,
    fetcher,
  );
}
