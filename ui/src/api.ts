export interface Liveness {
  status: "alive";
}

export type SourceMediaProfile =
  | { kind: "alsa_l16"; device: string }
  | {
      kind: "srt_mpeg_ts";
      host: string;
      port: number;
      mode: "caller" | "listener";
    }
  | { kind: "rtp_mp2t"; bindAddress: string; port: number }
  | {
      kind: "rtp_l16";
      bindAddress: string;
      port: number;
      payloadType: number;
    }
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
    | "rtp_l16_to_rtp_l16"
    | "rtp_l24_to_rtp_l24"
    | "rtp_mp2t_to_srt_mpeg_ts"
    | "srt_mpeg_ts_to_srt_mpeg_ts";
  estimate?: {
    capacityUnits: number;
    ingressPayloadBitsPerSecond: number | null;
    egressPayloadBitsPerSecond: number | null;
  };
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

export function updateSource(
  sourceId: string,
  friendlyName: string,
  profile: SourceMediaProfile,
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<SourceRecord> {
  return requestJson(
    `/sources/${encodeURIComponent(sourceId)}`,
    {
      method: "PUT",
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

export function updateDestination(
  destinationId: string,
  friendlyName: string,
  profile: DestinationMediaProfile,
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<DestinationRecord> {
  return requestJson(
    `/destinations/${encodeURIComponent(destinationId)}`,
    {
      method: "PUT",
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

export async function planRoute(
  source: SourceMediaProfile,
  destination: DestinationMediaProfile,
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<RoutePlan> {
  const response = await requestJson<unknown>(
    "/routes/plan",
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ source, destination }),
    },
    baseUrl,
    fetcher,
  );
  if (
    typeof response !== "object" ||
    response === null ||
    !("mediaPath" in response) ||
    !isMediaPath(response.mediaPath)
  ) {
    throw new Error("API returned an invalid route plan");
  }
  const mediaPath = response.mediaPath;
  if (!("estimate" in response)) {
    return { mediaPath };
  }
  const estimate = response.estimate;
  if (
    typeof estimate !== "object" ||
    estimate === null ||
    !("capacityUnits" in estimate) ||
    !isNonnegativeInteger(estimate.capacityUnits) ||
    estimate.capacityUnits === 0 ||
    !("ingressPayloadBitsPerSecond" in estimate) ||
    !isPayloadRate(estimate.ingressPayloadBitsPerSecond) ||
    !("egressPayloadBitsPerSecond" in estimate) ||
    !isPayloadRate(estimate.egressPayloadBitsPerSecond)
  ) {
    throw new Error("API returned an invalid route estimate");
  }
  return {
    mediaPath,
    estimate: {
      capacityUnits: estimate.capacityUnits,
      ingressPayloadBitsPerSecond: estimate.ingressPayloadBitsPerSecond,
      egressPayloadBitsPerSecond: estimate.egressPayloadBitsPerSecond,
    },
  };
}

function isMediaPath(value: unknown): value is RoutePlan["mediaPath"] {
  return (
    value === "alsa_l16_to_rtp_l16" ||
    value === "rtp_l16_to_rtp_l16" ||
    value === "rtp_l24_to_rtp_l24" ||
    value === "rtp_mp2t_to_srt_mpeg_ts" ||
    value === "srt_mpeg_ts_to_srt_mpeg_ts"
  );
}

function isNonnegativeInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function isPayloadRate(value: unknown): value is number | null {
  return value === null || isNonnegativeInteger(value);
}
