export interface Liveness {
  status: "alive";
}

export interface AdminAuthStatus {
  bootstrap_required: boolean;
  login_required: boolean;
}

export interface AdminSession {
  username: string;
  csrf_token: string;
}

export interface RecoveryResult extends AdminSession {
  replacement_recovery_token: string;
  recovery_token_warning: string;
}

export interface Readiness {
  status: "ready";
  database: "ready";
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

export interface RouteRecord {
  destinationId: string;
  sourceId: string;
  desiredState: string;
  observedState: string;
  observedSourceId?: string | null;
  revision: number;
}

export interface SetDesiredRouteInput {
  sourceId: string;
  destinationId: string;
  expectedRouteRevision: number;
  confirmReplace: boolean;
}

export interface RouteBatchOperation {
  operationId: string;
  state:
    | "desired_state_applied"
    | "pending"
    | "running"
    | "completed"
    | "partial"
    | "failed";
  routes: RouteRecord[];
  results?: RouteBatchItemResult[];
}

export interface PresetRoute {
  sourceId: string;
  destinationId: string;
  action?: "start" | "stop";
}

export interface RoutePreset {
  id: string;
  name: string;
  revision: number;
  routes: PresetRoute[];
}

export interface RouteBatchItemResult {
  sourceId: string;
  destinationId: string;
  pipelineId: string;
  state: "pending" | "processing" | "succeeded" | "failed";
  assignmentId: string | null;
  assignmentGeneration: number | null;
  errorCode: string | null;
  errorMessage: string | null;
  action?: PresetRoute["action"];
}

export interface PlannedAssignment {
  id: string;
  pipelineId: string;
  workerId: string;
  generation: number;
}

export interface AssignmentSnapshot {
  assignment: PlannedAssignment;
  desiredState: string;
  observedState: string;
}

export interface CancelAssignmentResult {
  assignmentId: string;
  generation: number;
  state: "cancelled";
}

export interface PlanRouteAssignmentResult {
  state: "planned" | "unavailable" | "waiting" | "cancelled";
  mediaPath: RoutePlan["mediaPath"];
  estimate: NonNullable<RoutePlan["estimate"]>;
  assignment: PlannedAssignment | null;
  rejections: { workerId: string; reason: string }[];
  requestId?: string;
  expiresAt?: string;
  reason?: string;
}

export interface PlanRouteAssignmentRequest {
  pipelineId: string;
  sourceId: string;
  sourceRevision: number;
  destinationId: string;
  destinationRevision: number;
  allowWaiting?: boolean;
}

export interface PlanRouteAssignmentInput extends PlanRouteAssignmentRequest {
  idempotencyKey: string;
  capacityRequestId?: string;
}

export class ApiRequestError extends Error {
  constructor(readonly status: number, detail?: string, readonly code?: string) {
    super(`API request failed with HTTP ${status}${detail ? `: ${detail}` : ""}`);
    this.name = "ApiRequestError";
  }
}

async function readApiError(response: Response): Promise<ApiRequestError> {
  let text: string;
  try {
    text = await response.text();
  } catch (error) {
    if (!(error instanceof TypeError || error instanceof DOMException)) throw error;
    return new ApiRequestError(
      response.status,
      `The API error response could not be read: ${error.message}`,
    );
  }
  let body: unknown;
  try {
    body = JSON.parse(text);
  } catch (error) {
    if (!(error instanceof SyntaxError)) throw error;
    const detail = response.headers.get("content-type")?.startsWith("text/plain")
      ? text.trim().slice(0, 500)
      : undefined;
    return new ApiRequestError(
      response.status,
      detail || "The API returned an invalid error response",
    );
  }
  if (
    typeof body === "object" && body !== null &&
    "code" in body && typeof body.code === "string" &&
    "message" in body && typeof body.message === "string" && body.message.trim()
  ) {
    return new ApiRequestError(response.status, body.message, body.code);
  }
  return new ApiRequestError(response.status, "The API returned an invalid error response");
}

async function deleteCatalogResource(
  path: "sources" | "destinations",
  id: string,
  expectedRevision: number,
  baseUrl: string,
  fetcher: typeof fetch,
): Promise<void> {
  const result = await requestJson<unknown>(
    `/${path}/${encodeURIComponent(id)}`,
    {
      method: "DELETE",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ expectedRevision }),
    },
    baseUrl,
    fetcher,
  );
  if (
    typeof result !== "object" || result === null ||
    !("deleted" in result) || result.deleted !== true
  ) {
    throw new Error("The API did not confirm resource deletion");
  }
}

export function deleteSource(
  id: string,
  revision: number,
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<void> {
  return deleteCatalogResource("sources", id, revision, baseUrl, fetcher);
}

export function deleteDestination(
  id: string,
  revision: number,
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<void> {
  return deleteCatalogResource("destinations", id, revision, baseUrl, fetcher);
}

let csrfToken: string | undefined;

export function setCsrfToken(value: string | undefined): void {
  csrfToken = value;
}

async function requestJson<T>(
  path: string,
  init: RequestInit | undefined,
  baseUrl: string,
  fetcher: typeof fetch,
): Promise<T> {
  const headers = new Headers(init?.headers);
  const method = init?.method?.toUpperCase() ?? "GET";
  let requestInit = init;
  if (csrfToken) {
    if (method !== "GET" && method !== "HEAD" && method !== "OPTIONS") {
      headers.set("x-arc-csrf-token", csrfToken);
    }
    requestInit = {
      ...init,
      credentials: "same-origin",
      headers,
    };
  }
  const response = await fetcher(`${baseUrl}/api/v1${path}`, requestInit);
  if (!response.ok) {
    if (response.status === 401 && typeof window !== "undefined") {
      window.dispatchEvent(new Event("arc-auth-required"));
    }
    throw await readApiError(response);
  }
  return (await response.json()) as T;
}

async function authRequest<T>(
  path: string,
  init?: RequestInit,
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<T> {
  const response = await fetcher(`${baseUrl}/api/v1/auth${path}`, {
    ...init,
    credentials: "same-origin",
  });
  if (!response.ok) {
    throw await readApiError(response);
  }
  return (await response.json()) as T;
}

export function fetchAdminAuthStatus(
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<AdminAuthStatus> {
  return authRequest("/status", undefined, baseUrl, fetcher);
}

export function fetchAdminSession(
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<AdminSession> {
  return authRequest("/session", undefined, baseUrl, fetcher);
}

export function loginAdmin(
  username: string,
  password: string,
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<{ csrf_token: string }> {
  return authRequest(
    "/login",
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ username, password }),
    },
    baseUrl,
    fetcher,
  );
}

export function bootstrapAdmin(
  bootstrapToken: string,
  username: string,
  password: string,
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<{ csrf_token: string }> {
  return authRequest(
    "/bootstrap",
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        bootstrap_token: bootstrapToken,
        username,
        password,
      }),
    },
    baseUrl,
    fetcher,
  );
}

export function recoverAdmin(
  recoveryToken: string,
  newPassword: string,
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<RecoveryResult> {
  return authRequest(
    "/recover",
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ recovery_token: recoveryToken, new_password: newPassword }),
    },
    baseUrl,
    fetcher,
  );
}

export function logoutAdmin(
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<{ ok: true }> {
  return authRequest(
    "/logout",
    {
      method: "POST",
      headers: { "x-arc-csrf-token": csrfToken ?? "" },
    },
    baseUrl,
    fetcher,
  );
}

export function changeAdminPassword(
  currentPassword: string,
  newPassword: string,
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<{ ok: true }> {
  return requestJson(
    "/auth/password",
    {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        current_password: currentPassword,
        new_password: newPassword,
      }),
    },
    baseUrl,
    fetcher,
  );
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

export async function fetchReadiness(
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<Readiness> {
  const response = await fetcher(`${baseUrl}/api/v1/health/ready`);
  if (!response.ok) {
    throw new Error(`API readiness request failed with HTTP ${response.status}`);
  }
  const body: unknown = await response.json();
  if (
    typeof body !== "object" ||
    body === null ||
    !("status" in body) ||
    body.status !== "ready" ||
    !("database" in body) ||
    body.database !== "ready"
  ) {
    throw new Error("API returned an invalid readiness response");
  }
  return { status: "ready", database: "ready" };
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

export async function fetchRoutes(
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<RouteRecord[]> {
  const response = await requestJson<unknown>(
    "/routes",
    undefined,
    baseUrl,
    fetcher,
  );
  if (
    typeof response !== "object" ||
    response === null ||
    !("items" in response) ||
    !Array.isArray(response.items) ||
    !response.items.every(isRouteRecord)
  ) {
    throw new Error("API returned an invalid route collection");
  }
  return response.items;
}

export async function setDesiredRoute(
  input: SetDesiredRouteInput,
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<RouteRecord> {
  const response = await requestJson<unknown>(
    "/routes",
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(input),
    },
    baseUrl,
    fetcher,
  );
  if (!isRouteRecord(response)) {
    throw new Error("API returned an invalid route");
  }
  return response;
}

export async function setDesiredRoutesBatch(
  routes: SetDesiredRouteInput[],
  idempotencyKey: string,
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<RouteBatchOperation> {
  const response = await requestJson<unknown>(
    "/routes/batch",
    {
      method: "POST",
      headers: {
        "content-type": "application/json",
        "idempotency-key": idempotencyKey,
      },
      body: JSON.stringify({ routes }),
    },
    baseUrl,
    fetcher,
  );
  if (!isRouteBatchOperation(response)) {
    throw new Error("API returned an invalid route batch operation");
  }
  return response;
}

export async function getRouteBatchOperation(
  operationId: string,
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<RouteBatchOperation> {
  const response = await requestJson<unknown>(
    `/operations/routes/${encodeURIComponent(operationId)}`,
    {},
    baseUrl,
    fetcher,
  );
  if (!isRouteBatchOperation(response)) {
    throw new Error("API returned an invalid route batch operation");
  }
  return response;
}

export async function fetchPresets(
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<RoutePreset[]> {
  const response = await requestJson<unknown>("/presets", undefined, baseUrl, fetcher);
  if (!Array.isArray(response) || !response.every(isRoutePreset)) {
    throw new Error("API returned an invalid salvo collection");
  }
  return response;
}

export async function savePreset(
  input: { name: string; routes: PresetRoute[]; id?: string; expectedRevision?: number },
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<RoutePreset> {
  const { id, ...body } = input;
  const response = await requestJson<unknown>(
    id ? `/presets/${encodeURIComponent(id)}` : "/presets",
    {
      method: id ? "PUT" : "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    },
    baseUrl,
    fetcher,
  );
  if (!isRoutePreset(response)) {
    throw new Error("API returned an invalid salvo");
  }
  return response;
}

export async function deletePreset(
  id: string,
  expectedRevision: number,
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<void> {
  const response = await requestJson<unknown>(
    `/presets/${encodeURIComponent(id)}`,
    {
      method: "DELETE",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ expectedRevision }),
    },
    baseUrl,
    fetcher,
  );
  if (typeof response !== "object" || response === null ||
      !("deleted" in response) || response.deleted !== true) {
    throw new Error("API did not confirm salvo deletion");
  }
}

export async function executePreset(
  id: string,
  input: { expectedRevision: number; routes: (SetDesiredRouteInput & PresetRoute)[] },
  idempotencyKey: string,
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<RouteBatchOperation> {
  const response = await requestJson<unknown>(
    `/presets/${encodeURIComponent(id)}/execute`,
    {
      method: "POST",
      headers: { "content-type": "application/json", "idempotency-key": idempotencyKey },
      body: JSON.stringify(input),
    },
    baseUrl,
    fetcher,
  );
  if (!isRouteBatchOperation(response)) {
    throw new Error("API returned an invalid salvo operation");
  }
  return response;
}

export async function unrouteDestination(
  input: { destinationId: string; expectedRouteRevision: number },
  idempotencyKey: string,
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<RouteBatchOperation> {
  const response = await requestJson<unknown>(
    "/routes/unroute",
    {
      method: "POST",
      headers: { "content-type": "application/json", "idempotency-key": idempotencyKey },
      body: JSON.stringify(input),
    },
    baseUrl,
    fetcher,
  );
  if (!isRouteBatchOperation(response)) {
    throw new Error("API returned an invalid unroute operation");
  }
  return response;
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

export function planRouteAssignment(
  input: PlanRouteAssignmentInput,
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<PlanRouteAssignmentResult> {
  const { idempotencyKey, capacityRequestId: _capacityRequestId, ...request } = input;
  return requestJson(
    "/routes/plan-assignment",
    {
      method: "POST",
      headers: {
        "content-type": "application/json",
        "Idempotency-Key": idempotencyKey,
      },
      body: JSON.stringify(request),
    },
    baseUrl,
    fetcher,
  );
}

export function fetchCapacityRequest(
  requestId: string,
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<PlanRouteAssignmentResult> {
  return requestJson(
    `/capacity-requests/${encodeURIComponent(requestId)}`,
    undefined,
    baseUrl,
    fetcher,
  );
}

export function cancelCapacityRequest(
  requestId: string,
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<PlanRouteAssignmentResult> {
  return requestJson(
    `/capacity-requests/${encodeURIComponent(requestId)}/cancel`,
    { method: "POST" },
    baseUrl,
    fetcher,
  );
}

export function fetchPipelineAssignment(
  pipelineId: string,
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<AssignmentSnapshot> {
  return requestJson(
    `/pipelines/${encodeURIComponent(pipelineId)}/assignment`,
    undefined,
    baseUrl,
    fetcher,
  );
}

export function startAssignment(
  assignmentId: string,
  generation: number,
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<AssignmentSnapshot> {
  return postAssignmentLifecycle(
    "start",
    assignmentId,
    generation,
    baseUrl,
    fetcher,
  );
}

export function stopAssignment(
  assignmentId: string,
  generation: number,
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<AssignmentSnapshot> {
  return postAssignmentLifecycle(
    "stop",
    assignmentId,
    generation,
    baseUrl,
    fetcher,
  );
}

export function cancelAssignment(
  assignmentId: string,
  generation: number,
  baseUrl = "",
  fetcher: typeof fetch = globalThis.fetch,
): Promise<CancelAssignmentResult> {
  return requestJson(
    `/assignments/${encodeURIComponent(assignmentId)}/cancel`,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ generation }),
    },
    baseUrl,
    fetcher,
  );
}

function postAssignmentLifecycle(
  action: "start" | "stop",
  assignmentId: string,
  generation: number,
  baseUrl: string,
  fetcher: typeof fetch,
): Promise<AssignmentSnapshot> {
  return requestJson(
    `/assignments/${encodeURIComponent(assignmentId)}/${action}`,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ generation }),
    },
    baseUrl,
    fetcher,
  );
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

function isRoutePreset(value: unknown): value is RoutePreset {
  return typeof value === "object" && value !== null &&
    "id" in value && typeof value.id === "string" &&
    "name" in value && typeof value.name === "string" &&
    "revision" in value && isNonnegativeInteger(value.revision) && value.revision > 0 &&
    "routes" in value && Array.isArray(value.routes) &&
    value.routes.length > 0 && value.routes.length <= 64 &&
    value.routes.every((route: unknown) =>
      typeof route === "object" && route !== null &&
      "sourceId" in route && typeof route.sourceId === "string" &&
      "destinationId" in route && typeof route.destinationId === "string" &&
      (!("action" in route) || route.action === "start" || route.action === "stop"));
}

function isPayloadRate(value: unknown): value is number | null {
  return value === null || isNonnegativeInteger(value);
}

function isRouteRecord(value: unknown): value is RouteRecord {
  return (
    typeof value === "object" &&
    value !== null &&
    "destinationId" in value &&
    typeof value.destinationId === "string" &&
    "sourceId" in value &&
    typeof value.sourceId === "string" &&
    "desiredState" in value &&
    typeof value.desiredState === "string" &&
    "observedState" in value &&
    typeof value.observedState === "string" &&
    (!("observedSourceId" in value) ||
      value.observedSourceId === null ||
      typeof value.observedSourceId === "string") &&
    "revision" in value &&
    typeof value.revision === "number" &&
    Number.isSafeInteger(value.revision) &&
    value.revision >= 0
  );
}

function isRouteBatchOperation(value: unknown): value is RouteBatchOperation {
  return (
    typeof value === "object" &&
    value !== null &&
    "operationId" in value &&
    typeof value.operationId === "string" &&
    "state" in value &&
    [
      "desired_state_applied",
      "pending",
      "running",
      "completed",
      "partial",
      "failed",
    ].includes(String(value.state)) &&
    "routes" in value &&
    Array.isArray(value.routes) &&
    value.routes.every(isRouteRecord) &&
    (!("results" in value) ||
      (Array.isArray(value.results) && value.results.every(isRouteBatchItemResult)))
  );
}

function isRouteBatchItemResult(value: unknown): value is RouteBatchItemResult {
  return (
    typeof value === "object" &&
    value !== null &&
    "sourceId" in value &&
    typeof value.sourceId === "string" &&
    "destinationId" in value &&
    typeof value.destinationId === "string" &&
    "pipelineId" in value &&
    typeof value.pipelineId === "string" &&
    (!("action" in value) || value.action === "start" || value.action === "stop") &&
    "state" in value &&
    ["pending", "processing", "succeeded", "failed"].includes(
      String(value.state),
    ) &&
    "assignmentId" in value &&
    (typeof value.assignmentId === "string" || value.assignmentId === null) &&
    "assignmentGeneration" in value &&
    (typeof value.assignmentGeneration === "number" ||
      value.assignmentGeneration === null) &&
    "errorCode" in value &&
    (typeof value.errorCode === "string" || value.errorCode === null) &&
    "errorMessage" in value &&
    (typeof value.errorMessage === "string" || value.errorMessage === null)
  );
}
