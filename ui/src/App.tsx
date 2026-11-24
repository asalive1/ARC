import { useMemo, useState, type FormEvent } from "react";
import {
  useMutation,
  useQuery,
  useQueryClient,
  type UseMutationResult,
} from "@tanstack/react-query";
import {
  Alert,
  Box,
  Button,
  Card,
  CardContent,
  Chip,
  CssBaseline,
  Divider,
  FormControlLabel,
  MenuItem,
  Paper,
  Stack,
  Switch,
  TextField,
  ThemeProvider,
  Typography,
  createTheme,
} from "@mui/material";

import {
  ApiRequestError,
  cancelAssignment as cancelAssignmentRequest,
  createDestination as createDestinationRequest,
  createSource as createSourceRequest,
  fetchDestinations,
  fetchLiveness,
  fetchPipelineAssignment,
  fetchSources,
  planRoute,
  planRouteAssignment,
  startAssignment as startAssignmentRequest,
  stopAssignment as stopAssignmentRequest,
  updateDestination as updateDestinationRequest,
  updateSource as updateSourceRequest,
  validateDestination,
  validateSource,
  type DestinationMediaProfile,
  type DestinationRecord,
  type PlanRouteAssignmentInput,
  type RoutePlan,
  type SourceMediaProfile,
  type SourceRecord,
} from "./api";

const apiBaseUrl = import.meta.env.VITE_ARC_API_BASE_URL ?? "";

type Section = "workspace" | "inputs" | "outputs" | "gpio" | "admin";

const navigation: { id: Section; label: string; badge?: string }[] = [
  { id: "workspace", label: "Crosspoint Grid", badge: "Planned" },
  { id: "inputs", label: "Inputs" },
  { id: "outputs", label: "Outputs" },
  { id: "gpio", label: "GPIO Configuration", badge: "Planned" },
  { id: "admin", label: "Admin Settings" },
];

const sectionTitles: Record<Section, string> = {
  workspace: "Crosspoint Grid",
  inputs: "Input Configuration",
  outputs: "Output Configuration",
  gpio: "GPIO Configuration",
  admin: "Admin Settings",
};

const sectionDescriptions: Record<Section, string> = {
  workspace: "Route connections and signal flow for this workspace.",
  inputs: "Configure and validate audio sources.",
  outputs: "Configure and validate audio destinations.",
  gpio: "Configure GPIO devices and signal mappings.",
  admin: "Workspace appearance and service status.",
};

function createPipelineId(): string {
  const bytes = globalThis.crypto.getRandomValues(new Uint8Array(16));
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0"))
    .join("");
  return [
    hex.slice(0, 8),
    hex.slice(8, 12),
    hex.slice(12, 16),
    hex.slice(16, 20),
    hex.slice(20),
  ].join("-");
}

export function App() {
  const queryClient = useQueryClient();
  const [section, setSection] = useState<Section>("workspace");
  const [themeMode, setThemeMode] = useState<"dark" | "light">("dark");
  const [sourceName, setSourceName] = useState("");
  const [editingSourceId, setEditingSourceId] = useState<string>();
  const [sourceKind, setSourceKind] =
    useState<SourceMediaProfile["kind"]>("srt_mpeg_ts");
  const [sourceDevice, setSourceDevice] = useState("hw:1,0");
  const [sourceHost, setSourceHost] = useState("127.0.0.1");
  const [sourceBindAddress, setSourceBindAddress] = useState("0.0.0.0");
  const [sourcePort, setSourcePort] = useState("5000");
  const [sourcePayloadType, setSourcePayloadType] = useState("96");
  const [sourceMode, setSourceMode] = useState<"caller" | "listener">(
    "caller",
  );
  const [destinationName, setDestinationName] = useState("");
  const [editingDestinationId, setEditingDestinationId] = useState<string>();
  const [destinationKind, setDestinationKind] =
    useState<DestinationMediaProfile["kind"]>("srt_mpeg_ts");
  const [destinationAddress, setDestinationAddress] =
    useState("127.0.0.1");
  const [destinationPort, setDestinationPort] = useState("6000");
  const [destinationPayloadType, setDestinationPayloadType] = useState("96");
  const [destinationMode, setDestinationMode] =
    useState<"caller" | "listener">("caller");
  const [sourceId, setSourceId] = useState("");
  const [destinationId, setDestinationId] = useState("");
  const [assignmentRequest, setAssignmentRequest] =
    useState<PlanRouteAssignmentInput>();

  const theme = useMemo(
    () =>
      createTheme({
        palette: {
          mode: themeMode,
          primary: { main: themeMode === "dark" ? "#4ea8ff" : "#1769aa" },
          background:
            themeMode === "dark"
              ? { default: "#0d151c", paper: "#131f28" }
              : { default: "#f3f6f9", paper: "#ffffff" },
          divider: themeMode === "dark" ? "#24323d" : "#dce3e9",
        },
        shape: { borderRadius: 8 },
        typography: {
          fontFamily:
            '"Segoe UI", "Inter", "Roboto", "Helvetica Neue", sans-serif',
        },
        components: {
          MuiCard: {
            styleOverrides: {
              root: ({ theme }) => ({
                backgroundImage: "none",
                border: `1px solid ${theme.palette.divider}`,
              }),
            },
          },
          MuiButton: {
            styleOverrides: { root: { textTransform: "none" } },
          },
        },
      }),
    [themeMode],
  );

  const health = useQuery({
    queryKey: ["api", "liveness"],
    queryFn: () => fetchLiveness(apiBaseUrl),
    retry: 1,
  });
  const sources = useQuery({
    queryKey: ["sources"],
    queryFn: () => fetchSources(apiBaseUrl),
  });
  const destinations = useQuery({
    queryKey: ["destinations"],
    queryFn: () => fetchDestinations(apiBaseUrl),
  });
  const createSource = useMutation({
    mutationFn: (input: {
      friendlyName: string;
      profile: SourceMediaProfile;
    }) => createSourceRequest(input.friendlyName, input.profile, apiBaseUrl),
    onSuccess: async () => {
      setSourceName("");
      await queryClient.invalidateQueries({ queryKey: ["sources"] });
    },
  });
  const createDestination = useMutation({
    mutationFn: (input: {
      friendlyName: string;
      profile: DestinationMediaProfile;
    }) =>
      createDestinationRequest(input.friendlyName, input.profile, apiBaseUrl),
    onSuccess: async () => {
      setDestinationName("");
      await queryClient.invalidateQueries({ queryKey: ["destinations"] });
    },
  });
  const updateSource = useMutation({
    mutationFn: (input: {
      id: string;
      friendlyName: string;
      profile: SourceMediaProfile;
    }) =>
      updateSourceRequest(
        input.id,
        input.friendlyName,
        input.profile,
        apiBaseUrl,
      ),
    onSuccess: async () => {
      resetSourceForm();
      await queryClient.invalidateQueries({ queryKey: ["sources"] });
    },
  });
  const updateDestination = useMutation({
    mutationFn: (input: {
      id: string;
      friendlyName: string;
      profile: DestinationMediaProfile;
    }) =>
      updateDestinationRequest(
        input.id,
        input.friendlyName,
        input.profile,
        apiBaseUrl,
      ),
    onSuccess: async () => {
      resetDestinationForm();
      await queryClient.invalidateQueries({ queryKey: ["destinations"] });
    },
  });
  const sourceValidation = useMutation({
    mutationFn: (id: string) => validateSource(id, apiBaseUrl),
    onSuccess: async () =>
      queryClient.invalidateQueries({ queryKey: ["sources"] }),
  });
  const destinationValidation = useMutation({
    mutationFn: (id: string) => validateDestination(id, apiBaseUrl),
    onSuccess: async () =>
      queryClient.invalidateQueries({ queryKey: ["destinations"] }),
  });
  const routePlan = useMutation({
    mutationFn: ({
      source,
      destination,
    }: {
      source: SourceMediaProfile;
      destination: DestinationMediaProfile;
    }) => planRoute(source, destination, apiBaseUrl),
  });

  const selectedSource =
    sources.data?.find((source) => source.id === sourceId) ??
    sources.data?.[0];
  const selectedDestination =
    destinations.data?.find(
      (destination) => destination.id === destinationId,
    ) ?? destinations.data?.[0];

  function beginEdit(record: SourceRecord | DestinationRecord) {
    createSource.reset();
    createDestination.reset();
    updateSource.reset();
    updateDestination.reset();

    if ("sourceType" in record) {
      setEditingSourceId(record.id);
      setSourceName(record.friendlyName);
      setSourceKind(record.configuration.kind);
      if (record.configuration.kind === "alsa_l16") {
        setSourceDevice(record.configuration.device);
      } else if (record.configuration.kind === "srt_mpeg_ts") {
        setSourceHost(record.configuration.host);
        setSourcePort(String(record.configuration.port));
        setSourceMode(record.configuration.mode);
      } else {
        setSourceBindAddress(record.configuration.bindAddress);
        setSourcePort(String(record.configuration.port));
        if (
          record.configuration.kind === "rtp_l16" ||
          record.configuration.kind === "rtp_l24"
        ) {
          setSourcePayloadType(String(record.configuration.payloadType));
        }
      }
      return;
    }

    setEditingDestinationId(record.id);
    setDestinationName(record.friendlyName);
    setDestinationKind(record.configuration.kind);
    setDestinationPort(String(record.configuration.port));
    if (record.configuration.kind === "srt_mpeg_ts") {
      setDestinationAddress(record.configuration.host);
      setDestinationMode(record.configuration.mode);
    } else {
      setDestinationAddress(record.configuration.address);
      setDestinationPayloadType(String(record.configuration.payloadType));
    }
  }

  function cancelSourceEdit() {
    resetSourceForm();
    createSource.reset();
    updateSource.reset();
  }

  function resetSourceForm() {
    setEditingSourceId(undefined);
    setSourceName("");
    setSourceKind("srt_mpeg_ts");
    setSourceDevice("hw:1,0");
    setSourceHost("127.0.0.1");
    setSourceBindAddress("0.0.0.0");
    setSourcePort("5000");
    setSourcePayloadType("96");
    setSourceMode("caller");
  }

  function cancelDestinationEdit() {
    resetDestinationForm();
    createDestination.reset();
    updateDestination.reset();
  }

  function resetDestinationForm() {
    setEditingDestinationId(undefined);
    setDestinationName("");
    setDestinationKind("srt_mpeg_ts");
    setDestinationAddress("127.0.0.1");
    setDestinationPort("6000");
    setDestinationPayloadType("96");
    setDestinationMode("caller");
  }

  function submitSource(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const profile: SourceMediaProfile = (() => {
      if (sourceKind === "alsa_l16") {
        return { kind: "alsa_l16", device: sourceDevice };
      }
      if (sourceKind === "srt_mpeg_ts") {
        return {
          kind: "srt_mpeg_ts",
          host: sourceHost,
          port: Number(sourcePort),
          mode: sourceMode,
        };
      }
      if (sourceKind === "rtp_l16") {
        return {
          kind: "rtp_l16",
          bindAddress: sourceBindAddress,
          port: Number(sourcePort),
          payloadType: Number(sourcePayloadType),
        };
      }
      if (sourceKind === "rtp_l24") {
        return {
          kind: "rtp_l24",
          bindAddress: sourceBindAddress,
          port: Number(sourcePort),
          payloadType: Number(sourcePayloadType),
        };
      }
      return {
        kind: "rtp_mp2t",
        bindAddress: sourceBindAddress,
        port: Number(sourcePort),
      };
    })();
    if (editingSourceId) {
      updateSource.mutate({
        id: editingSourceId,
        friendlyName: sourceName,
        profile,
      });
    } else {
      createSource.mutate({ friendlyName: sourceName, profile });
    }
  }

  function submitDestination(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const profile: DestinationMediaProfile = (() => {
      if (destinationKind === "srt_mpeg_ts") {
        return {
          kind: "srt_mpeg_ts",
          host: destinationAddress,
          port: Number(destinationPort),
          mode: destinationMode,
        };
      }
      return {
        kind: destinationKind,
        address: destinationAddress,
        port: Number(destinationPort),
        payloadType: Number(destinationPayloadType),
      };
    })();
    if (editingDestinationId) {
      updateDestination.mutate({
        id: editingDestinationId,
        friendlyName: destinationName,
        profile,
      });
    } else {
      createDestination.mutate({ friendlyName: destinationName, profile });
    }
  }

  function submitRoutePlan(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (selectedSource && selectedDestination) {
      routePlan.mutate({
        source: selectedSource.configuration,
        destination: selectedDestination.configuration,
      });
    }
  }

  const healthLabel = health.isPending
    ? "Checking API"
    : health.isError
      ? "API unavailable"
      : "API online";

  return (
    <ThemeProvider theme={theme}>
      <CssBaseline />
      <Box
        sx={{
          display: "flex",
          minHeight: "100vh",
          bgcolor: "background.default",
          color: "text.primary",
        }}
      >
        <Box
          component="aside"
          sx={{
            width: { xs: 188, md: 218 },
            flexShrink: 0,
            display: "flex",
            flexDirection: "column",
            borderRight: 1,
            borderColor: "divider",
            bgcolor: "background.paper",
          }}
        >
          <Stack
            direction="row"
            spacing={1.25}
            alignItems="center"
            sx={{ minHeight: 64, px: 2, borderBottom: 1, borderColor: "divider" }}
          >
            <Box
              aria-hidden="true"
              sx={{
                width: 30,
                height: 30,
                borderRadius: 1.5,
                display: "grid",
                placeItems: "center",
                bgcolor: "primary.main",
                color: "#07131c",
                fontSize: 17,
                fontWeight: 800,
              }}
            >
              A
            </Box>
            <Box>
              <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>
                ARC
              </Typography>
              <Typography variant="caption" color="text.secondary">
                Audio Routing Controller
              </Typography>
            </Box>
          </Stack>

          <Stack spacing={0.5} sx={{ px: 1.25, py: 2, flex: 1 }}>
            <Typography
              variant="overline"
              color="text.secondary"
              sx={{ px: 1.25, letterSpacing: 1.2 }}
            >
              Workspace
            </Typography>
            {navigation.slice(0, 1).map((item) => (
              <NavigationItem
                key={item.id}
                label={item.label}
                badge={item.badge}
                selected={section === item.id}
                onClick={() => setSection(item.id)}
              />
            ))}

            <Typography
              variant="overline"
              color="text.secondary"
              sx={{ px: 1.25, pt: 2.5, letterSpacing: 1.2 }}
            >
              Management
            </Typography>
            {navigation.slice(1).map((item) => (
              <NavigationItem
                key={item.id}
                label={item.label}
                badge={item.badge}
                selected={section === item.id}
                onClick={() => setSection(item.id)}
              />
            ))}
          </Stack>

          <Divider />
          <Stack spacing={0.25} sx={{ p: 2 }}>
            <Typography variant="body2" sx={{ fontWeight: 600 }}>
              Local workspace
            </Typography>
            <Typography variant="caption" color="text.secondary">
              Authentication not configured
            </Typography>
          </Stack>
        </Box>

        <Box sx={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column" }}>
          <Box
            component="header"
            sx={{
              minHeight: 56,
              px: { xs: 2, md: 3 },
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
              gap: 2,
              borderBottom: 1,
              borderColor: "divider",
              bgcolor: "background.paper",
            }}
          >
            <Typography variant="body2" color="text.secondary">
              Local workspace <Box component="span" sx={{ px: 1 }}>›</Box>
              <Box component="span" sx={{ color: "text.primary" }}>
                {sectionTitles[section]}
              </Box>
            </Typography>
            <Stack direction="row" spacing={2} alignItems="center">
              <Stack direction="row" spacing={1} alignItems="center">
                <Box
                  aria-hidden="true"
                  sx={{
                    width: 7,
                    height: 7,
                    borderRadius: "50%",
                    bgcolor: health.isError
                      ? "error.main"
                      : health.isPending
                        ? "warning.main"
                        : "success.main",
                    boxShadow: health.isError
                      ? "0 0 10px rgba(239,83,80,.45)"
                      : "0 0 10px rgba(76,175,80,.45)",
                  }}
                />
                <Typography variant="caption">{healthLabel}</Typography>
              </Stack>
              <FormControlLabel
                sx={{ m: 0 }}
                control={
                  <Switch
                    size="small"
                    checked={themeMode === "dark"}
                    onChange={(event) =>
                      setThemeMode(event.target.checked ? "dark" : "light")
                    }
                    inputProps={{ "aria-label": "Use dark theme" }}
                  />
                }
                label={
                  <Typography variant="caption">
                    {themeMode === "dark" ? "Dark" : "Light"}
                  </Typography>
                }
              />
            </Stack>
          </Box>

          <Box
            component="main"
            sx={{
              flex: 1,
              width: "100%",
              maxWidth: 1600,
              mx: "auto",
              p: { xs: 2, md: 3.5 },
            }}
          >
            <Stack spacing={3}>
              <Stack
                direction={{ xs: "column", md: "row" }}
                alignItems={{ xs: "flex-start", md: "center" }}
                justifyContent="space-between"
                spacing={1}
              >
                <Box>
                  <Typography
                    variant="overline"
                    color="primary.main"
                    sx={{ letterSpacing: 1.3, fontWeight: 700 }}
                  >
                    Operations workspace
                  </Typography>
                  <Typography variant="h4" component="h1" sx={{ fontWeight: 650 }}>
                    {sectionTitles[section]}
                  </Typography>
                  <Typography color="text.secondary" sx={{ mt: 0.5 }}>
                    {sectionDescriptions[section]}
                  </Typography>
                </Box>
                {section === "inputs" && (
                  <Chip
                    size="small"
                    label={`${sources.data?.length ?? 0} inputs`}
                    variant="outlined"
                  />
                )}
                {section === "outputs" && (
                  <Chip
                    size="small"
                    label={`${destinations.data?.length ?? 0} outputs`}
                    variant="outlined"
                  />
                )}
              </Stack>

              {section === "workspace" && (
                <WorkspaceView
                  sources={sources.data ?? []}
                  destinations={destinations.data ?? []}
                  selectedSourceId={selectedSource?.id ?? ""}
                  selectedDestinationId={selectedDestination?.id ?? ""}
                  assignmentRequest={assignmentRequest}
                  onAssignmentRequestChange={setAssignmentRequest}
                  onSourceChange={setSourceId}
                  onDestinationChange={setDestinationId}
                  onPlan={submitRoutePlan}
                  routePlan={routePlan}
                  sourceError={sources.isError ? sources.error.message : ""}
                  destinationError={
                    destinations.isError ? destinations.error.message : ""
                  }
                />
              )}

              {section === "inputs" && (
                <ResourceWorkspace
                  kind="input"
                  loading={sources.isLoading}
                  error={sources.isError ? sources.error.message : ""}
                  records={sources.data ?? []}
                  saveError={
                    createSource.isError
                      ? createSource.error.message
                      : updateSource.isError
                        ? updateSource.error.message
                        : ""
                  }
                  editingId={editingSourceId}
                  onEdit={beginEdit}
                  onCancelEdit={cancelSourceEdit}
                  validationError={
                    sourceValidation.isError
                      ? sourceValidation.error.message
                      : ""
                  }
                  validatingId={
                    sourceValidation.isPending
                      ? sourceValidation.variables
                      : undefined
                  }
                  onValidate={(id) => sourceValidation.mutate(id)}
                  onSubmit={submitSource}
                  name={sourceName}
                  onNameChange={setSourceName}
                  profileKind={sourceKind}
                  onProfileKindChange={(kind) => setSourceKind(kind)}
                  sourceDevice={sourceDevice}
                  onSourceDeviceChange={setSourceDevice}
                  sourceHost={sourceHost}
                  onSourceHostChange={setSourceHost}
                  sourceBindAddress={sourceBindAddress}
                  onSourceBindAddressChange={setSourceBindAddress}
                  sourcePort={sourcePort}
                  onSourcePortChange={setSourcePort}
                  sourcePayloadType={sourcePayloadType}
                  onSourcePayloadTypeChange={setSourcePayloadType}
                  sourceMode={sourceMode}
                  onSourceModeChange={setSourceMode}
                  saving={createSource.isPending || updateSource.isPending}
                />
              )}

              {section === "outputs" && (
                <ResourceWorkspace
                  kind="output"
                  loading={destinations.isLoading}
                  error={destinations.isError ? destinations.error.message : ""}
                  records={destinations.data ?? []}
                  saveError={
                    createDestination.isError
                      ? createDestination.error.message
                      : updateDestination.isError
                        ? updateDestination.error.message
                        : ""
                  }
                  editingId={editingDestinationId}
                  onEdit={beginEdit}
                  onCancelEdit={cancelDestinationEdit}
                  validationError={
                    destinationValidation.isError
                      ? destinationValidation.error.message
                      : ""
                  }
                  validatingId={
                    destinationValidation.isPending
                      ? destinationValidation.variables
                      : undefined
                  }
                  onValidate={(id) => destinationValidation.mutate(id)}
                  onSubmit={submitDestination}
                  name={destinationName}
                  onNameChange={setDestinationName}
                  profileKind={destinationKind}
                  onProfileKindChange={(kind) =>
                    setDestinationKind(kind as DestinationMediaProfile["kind"])
                  }
                  destinationAddress={destinationAddress}
                  onDestinationAddressChange={setDestinationAddress}
                  destinationPort={destinationPort}
                  onDestinationPortChange={setDestinationPort}
                  destinationPayloadType={destinationPayloadType}
                  onDestinationPayloadTypeChange={setDestinationPayloadType}
                  destinationMode={destinationMode}
                  onDestinationModeChange={setDestinationMode}
                  saving={
                    createDestination.isPending || updateDestination.isPending
                  }
                />
              )}

              {section === "gpio" && <PlannedWorkspace title="GPIO configuration" />}
              {section === "admin" && (
                <AdminWorkspace
                  themeMode={themeMode}
                  onThemeChange={setThemeMode}
                  healthLabel={healthLabel}
                  healthy={health.isSuccess}
                  error={health.isError ? health.error.message : ""}
                />
              )}
            </Stack>
          </Box>

          <Box
            component="footer"
            sx={{
              minHeight: 36,
              px: { xs: 2, md: 3 },
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
              borderTop: 1,
              borderColor: "divider",
              color: "text.secondary",
            }}
          >
            <Typography variant="caption">ARC · Phase 1 preview</Typography>
            <Typography variant="caption">
              Assignment media status is shown in the workspace
            </Typography>
          </Box>
        </Box>
      </Box>
    </ThemeProvider>
  );
}

function NavigationItem({
  label,
  badge,
  selected,
  onClick,
}: {
  label: string;
  badge?: string;
  selected: boolean;
  onClick: () => void;
}) {
  return (
    <Button
      onClick={onClick}
      aria-current={selected ? "page" : undefined}
      sx={{
        minHeight: 38,
        px: 1.25,
        justifyContent: "flex-start",
        color: selected ? "primary.main" : "text.secondary",
        bgcolor: selected ? "action.selected" : "transparent",
        borderLeft: selected ? 2 : 2,
        borderColor: selected ? "primary.main" : "transparent",
        "&:hover": { bgcolor: "action.hover" },
      }}
    >
      <Box
        component="span"
        sx={{
          flex: 1,
          textAlign: "left",
          fontSize: 13,
          fontWeight: selected ? 600 : 400,
          whiteSpace: "nowrap",
        }}
      >
        {label}
      </Box>
      {badge && (
        <Typography variant="caption" color="text.disabled" sx={{ ml: 0.5 }}>
          {badge}
        </Typography>
      )}
    </Button>
  );
}

function WorkspaceView({
  sources,
  destinations,
  selectedSourceId,
  selectedDestinationId,
  assignmentRequest,
  onAssignmentRequestChange,
  onSourceChange,
  onDestinationChange,
  onPlan,
  routePlan,
  sourceError,
  destinationError,
}: {
  sources: SourceRecord[];
  destinations: DestinationRecord[];
  selectedSourceId: string;
  selectedDestinationId: string;
  assignmentRequest: PlanRouteAssignmentInput | undefined;
  onAssignmentRequestChange: (
    request: PlanRouteAssignmentInput | undefined,
  ) => void;
  onSourceChange: (id: string) => void;
  onDestinationChange: (id: string) => void;
  onPlan: (event: FormEvent<HTMLFormElement>) => void;
  routePlan: UseMutationResult<
    RoutePlan,
    Error,
    { source: SourceMediaProfile; destination: DestinationMediaProfile }
  >;
  sourceError: string;
  destinationError: string;
}) {
  const selectedSource = sources.find((source) => source.id === selectedSourceId);
  const selectedDestination = destinations.find(
    (destination) => destination.id === selectedDestinationId,
  );
  const queryClient = useQueryClient();
  const activePipelineId = assignmentRequest?.pipelineId;
  const resultMatchesSelection =
    selectedSource !== undefined &&
    selectedDestination !== undefined &&
    routePlan.variables !== undefined &&
    JSON.stringify(routePlan.variables.source) === JSON.stringify(selectedSource.configuration) &&
    JSON.stringify(routePlan.variables.destination) === JSON.stringify(selectedDestination.configuration);
  const assignmentPlan = useMutation({
    mutationFn: (input: PlanRouteAssignmentInput) =>
      planRouteAssignment(input, apiBaseUrl),
    onSuccess: (result, input) =>
      onAssignmentRequestChange(result.assignment ? input : undefined),
  });
  const pipelineAssignment = useQuery({
    queryKey: ["pipeline-assignment", activePipelineId],
    queryFn: () => {
      if (!activePipelineId) {
        throw new Error("Pipeline ID is required to fetch an assignment");
      }
      return fetchPipelineAssignment(activePipelineId, apiBaseUrl);
    },
    enabled:
      activePipelineId !== undefined &&
      !assignmentPlan.isPending &&
      (assignmentPlan.isIdle ||
        assignmentPlan.isError ||
        assignmentPlan.data?.assignment != null),
    refetchInterval: (query) =>
      query.state.data?.observedState === "starting" ||
      query.state.data?.observedState === "stopping"
        ? 1500
        : false,
  });
  const start = useMutation({
    mutationFn: (input: {
      pipelineId: string;
      assignmentId: string;
      generation: number;
    }) =>
      startAssignmentRequest(input.assignmentId, input.generation, apiBaseUrl),
    onSuccess: async (snapshot, input) => {
      queryClient.setQueryData(
        ["pipeline-assignment", input.pipelineId],
        snapshot,
      );
      await invalidateResourceRecords();
    },
    onError: async (_error, input) => {
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: ["pipeline-assignment", input.pipelineId],
        }),
        invalidateResourceRecords(),
      ]);
    },
  });
  const stop = useMutation({
    mutationFn: (input: {
      pipelineId: string;
      assignmentId: string;
      generation: number;
    }) =>
      stopAssignmentRequest(input.assignmentId, input.generation, apiBaseUrl),
    onSuccess: async (snapshot, input) => {
      queryClient.setQueryData(
        ["pipeline-assignment", input.pipelineId],
        snapshot,
      );
      await invalidateResourceRecords();
    },
    onError: async (_error, input) => {
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: ["pipeline-assignment", input.pipelineId],
        }),
        invalidateResourceRecords(),
      ]);
    },
  });
  const cancel = useMutation({
    mutationFn: (input: {
      pipelineId: string;
      assignmentId: string;
      generation: number;
    }) =>
      cancelAssignmentRequest(
        input.assignmentId,
        input.generation,
        apiBaseUrl,
      ),
    onSuccess: async (_result, input) => {
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: ["pipeline-assignment", input.pipelineId],
        }),
        invalidateResourceRecords(),
      ]);
    },
    onError: async (_error, input) => {
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: ["pipeline-assignment", input.pipelineId],
        }),
        invalidateResourceRecords(),
      ]);
    },
  });
  const assignmentVariables =
    assignmentPlan.variables ?? assignmentRequest;
  const assignmentPlanMatchesSelection =
    selectedSource !== undefined &&
    selectedDestination !== undefined &&
    assignmentVariables !== undefined &&
    assignmentVariables.sourceId === selectedSource.id &&
    assignmentVariables.sourceRevision === selectedSource.currentRevision &&
    assignmentVariables.destinationId === selectedDestination.id &&
    assignmentVariables.destinationRevision ===
      selectedDestination.currentRevision;
  const assignmentPlanResult = assignmentPlanMatchesSelection
    ? assignmentPlan.data
    : undefined;
  const currentAssignment =
    activePipelineId !== undefined &&
    pipelineAssignment.data?.assignment.pipelineId === activePipelineId
      ? pipelineAssignment.data
      : undefined;
  const currentAssignmentIsTerminal =
    currentAssignment !== undefined &&
    ((currentAssignment.desiredState === "stopped" &&
      currentAssignment.observedState === "stopped") ||
      (currentAssignment.desiredState === "cancelled" &&
        currentAssignment.observedState === "cancelled"));
  const assignmentNotFound =
    pipelineAssignment.error instanceof ApiRequestError &&
    pipelineAssignment.error.status === 404;
  const assignmentStatusUnknown =
    activePipelineId !== undefined &&
    pipelineAssignment.isError &&
    !assignmentNotFound &&
    assignmentPlan.data?.state !== "unavailable";
  const canCreateAssignment =
    selectedSource?.lifecycle === "validated" &&
    selectedDestination?.lifecycle === "validated" &&
    !assignmentPlan.isPending &&
    (activePipelineId === undefined ||
      assignmentPlan.data?.state === "unavailable" ||
      !pipelineAssignment.isFetching) &&
    !assignmentStatusUnknown &&
    (!currentAssignment || currentAssignmentIsTerminal) &&
    (assignmentPlanResult?.assignment == null ||
      currentAssignmentIsTerminal);

  const lifecycleActionPending =
    start.isPending || stop.isPending || cancel.isPending;

  async function invalidateResourceRecords() {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ["sources"] }),
      queryClient.invalidateQueries({ queryKey: ["destinations"] }),
    ]);
  }

  function submitAssignment() {
    if (
      !selectedSource ||
      !selectedDestination ||
      selectedSource.lifecycle !== "validated" ||
      selectedDestination.lifecycle !== "validated"
    ) {
      return;
    }

    const previousAttempt =
      assignmentPlan.variables ?? assignmentRequest;
    const retryingSamePlan =
      !currentAssignmentIsTerminal &&
      previousAttempt?.sourceId === selectedSource.id &&
      previousAttempt.sourceRevision === selectedSource.currentRevision &&
      previousAttempt.destinationId === selectedDestination.id &&
      previousAttempt.destinationRevision ===
        selectedDestination.currentRevision;
    const pipelineId = retryingSamePlan && previousAttempt
      ? previousAttempt.pipelineId
      : createPipelineId();
    const idempotencyKey = retryingSamePlan && previousAttempt
      ? previousAttempt.idempotencyKey
      : createPipelineId();
    const input = {
      pipelineId,
      idempotencyKey,
      sourceId: selectedSource.id,
      sourceRevision: selectedSource.currentRevision,
      destinationId: selectedDestination.id,
      destinationRevision: selectedDestination.currentRevision,
    };
    onAssignmentRequestChange(input);
    assignmentPlan.mutate(input);
  }

  return (
    <Stack spacing={2.5}>
      <Paper variant="outlined" sx={{ p: { xs: 2, md: 3 } }}>
        <Stack
          direction={{ xs: "column", md: "row" }}
          justifyContent="space-between"
          alignItems={{ xs: "flex-start", md: "center" }}
          spacing={2}
          sx={{ mb: 2.5 }}
        >
          <Box>
            <Stack direction="row" spacing={1} alignItems="center">
              <Typography variant="h6" component="h2">
                Routing matrix
              </Typography>
              <Chip
                size="small"
                label="Planned"
                color="warning"
                variant="outlined"
              />
            </Stack>
            <Typography variant="body2" color="text.secondary" sx={{ mt: 0.5 }}>
            The multi-route crosspoint editor remains planned. Create and
            manage a single validated assignment below.
            </Typography>
          </Box>
          <Chip label="Matrix editor planned" size="small" />
        </Stack>
        <Box
          role="img"
          aria-label="Decorative placeholder for the planned routing matrix"
          sx={{
            display: "grid",
            gridTemplateColumns: "repeat(8, minmax(24px, 1fr))",
            gap: 0.75,
            p: { xs: 1, md: 2 },
            border: 1,
            borderColor: "divider",
            borderRadius: 1,
            bgcolor: "background.default",
          }}
        >
          {Array.from({ length: 64 }, (_, index) => (
            <Box
              key={index}
              sx={{
                aspectRatio: "1 / 1",
                minHeight: 22,
                border: 1,
                borderColor: "divider",
                borderRadius: 0.5,
                bgcolor: "background.paper",
              }}
            />
          ))}
        </Box>
        <Stack
          direction={{ xs: "column", sm: "row" }}
          justifyContent="space-between"
          spacing={1}
          sx={{ mt: 1.5 }}
        >
          <Typography variant="caption" color="text.secondary">
            Matrix preview only · Crosspoints cannot be changed
          </Typography>
          <Typography variant="caption" color="text.secondary">
            Inputs {sources.length} · Outputs {destinations.length}
          </Typography>
        </Stack>
      </Paper>

      <Card>
        <CardContent>
          <Stack
            component="form"
            spacing={2}
            onSubmit={onPlan}
          >
            <Box>
              <Typography variant="h6" component="h2">
                Check media compatibility
              </Typography>
              <Typography variant="body2" color="text.secondary">
                Preview a supported path without creating or activating a route.
              </Typography>
            </Box>
            {sourceError && (
              <Alert severity="error">
                Inputs could not be loaded: {sourceError}
              </Alert>
            )}
            {destinationError && (
              <Alert severity="error">
                Outputs could not be loaded: {destinationError}
              </Alert>
            )}
            {sources.length === 0 || destinations.length === 0 ? (
              <Alert severity="info">
                Add at least one input and one output before checking a media
                path.
              </Alert>
            ) : (
              <Box
                sx={{
                  display: "grid",
                  gridTemplateColumns: { xs: "1fr", md: "1fr 1fr auto" },
                  gap: 2,
                  alignItems: "center",
                }}
              >
                <TextField
                  select
                  label="Input"
                  value={selectedSourceId}
                  onChange={(event) => onSourceChange(event.target.value)}
                  required
                >
                  {sources.map((source) => (
                    <MenuItem key={source.id} value={source.id}>
                      {source.friendlyName} · {source.configuration.kind}
                    </MenuItem>
                  ))}
                </TextField>
                <TextField
                  select
                  label="Output"
                  value={selectedDestinationId}
                  onChange={(event) => onDestinationChange(event.target.value)}
                  required
                >
                  {destinations.map((destination) => (
                    <MenuItem key={destination.id} value={destination.id}>
                      {destination.friendlyName} ·{" "}
                      {destination.configuration.kind}
                    </MenuItem>
                  ))}
                </TextField>
                <Button
                  type="submit"
                  variant="contained"
                  disabled={routePlan.isPending}
                >
                  Check path
                </Button>
              </Box>
            )}
            {routePlan.isError && resultMatchesSelection && (
              <Alert severity="error">
                Path check failed: {routePlan.error.message}
              </Alert>
            )}
            {routePlan.isSuccess && resultMatchesSelection && (
              <Alert severity="success">
                Compatible path: {routePlan.data.mediaPath}. This is a plan
                only; no worker capacity was checked or reserved, and no media
                is flowing.
                {routePlan.data.estimate ? (
                  <>
                    <br />
                    Provisional, uncalibrated estimate: {routePlan.data.estimate.capacityUnits} capacity
                    units. Payload rate: input{" "}
                    {routePlan.data.estimate.ingressPayloadBitsPerSecond === null
                      ? "unknown"
                      : `${routePlan.data.estimate.ingressPayloadBitsPerSecond.toLocaleString()} bits/s`}
                    ; output{" "}
                    {routePlan.data.estimate.egressPayloadBitsPerSecond === null
                      ? "unknown"
                      : `${routePlan.data.estimate.egressPayloadBitsPerSecond.toLocaleString()} bits/s`}.
                    Rates exclude protocol overhead.
                  </>
                ) : (
                  <>
                    <br />
                    Resource estimate unavailable from this API version.
                  </>
                )}
              </Alert>
            )}
          </Stack>
        </CardContent>
      </Card>

      <Card>
        <CardContent>
          <Stack spacing={2}>
            <Box>
              <Typography variant="h6" component="h2">
                Worker assignment
              </Typography>
              <Typography variant="body2" color="text.secondary">
                Reserve capacity for the selected validated input and output,
                then Start or Stop it; unstarted assignments can be cancelled.
                Planning alone does not start media.
              </Typography>
            </Box>
            {(selectedSource?.lifecycle !== "validated" ||
              selectedDestination?.lifecycle !== "validated") && (
              <Alert severity="info">
                Validate the selected input and output before assigning a
                worker.
              </Alert>
            )}
            {selectedSource && selectedDestination && (
              <Button
                variant="contained"
                onClick={submitAssignment}
                disabled={!canCreateAssignment}
              >
                {assignmentPlan.isPending
                  ? "Assigning…"
                  : assignmentPlanResult?.state === "unavailable"
                    ? "Retry worker assignment"
                    : "Reserve worker and plan"}
              </Button>
            )}
            {assignmentPlan.isError && assignmentPlanMatchesSelection && (
              <Alert
                severity="error"
                action={
                  <Button
                    color="inherit"
                    size="small"
                    onClick={() => void pipelineAssignment.refetch()}
                  >
                    Check status
                  </Button>
                }
              >
                Assignment request failed: {assignmentPlan.error.message}.{" "}
                {currentAssignment
                  ? "A persisted assignment was found and loaded."
                  : "The same pipeline ID is being checked for a committed assignment."}
                {pipelineAssignment.isError
                  ? ` Lookup failed: ${pipelineAssignment.error.message}`
                  : ""}
              </Alert>
            )}
            {assignmentPlanResult?.state === "unavailable" && (
              <Alert severity="warning">
                No eligible worker could be assigned.
                {assignmentPlanResult.rejections.length > 0
                  ? ` ${assignmentPlanResult.rejections
                      .map(
                        (rejection) =>
                          `${rejection.workerId}: ${rejection.reason}`,
                      )
                      .join("; ")}`
                  : " Check worker readiness and available capacity before retrying."}
              </Alert>
            )}
            {activePipelineId !== undefined &&
              pipelineAssignment.isError &&
              !assignmentPlan.isError &&
              assignmentPlanResult?.assignment == null && (
                <Alert
                  severity={assignmentStatusUnknown ? "error" : "info"}
                  action={
                    <Button
                      color="inherit"
                      size="small"
                      onClick={() => void pipelineAssignment.refetch()}
                    >
                      Check status
                    </Button>
                  }
                >
                  Could not confirm pipeline {activePipelineId}:{" "}
                  {pipelineAssignment.error.message}.{" "}
                  {assignmentStatusUnknown
                    ? "New assignments are blocked until status is confirmed."
                    : "Check status; if no assignment exists, retry using this pipeline ID."}
                </Alert>
              )}
            {assignmentPlanResult?.assignment && !currentAssignment && (
              <Alert
                severity={
                  pipelineAssignment.isError ? "error" : "info"
                }
              >
                {pipelineAssignment.isError
                  ? `Could not read assignment status: ${pipelineAssignment.error.message}`
                  : "Assignment created; loading its current status…"}
              </Alert>
            )}
            {currentAssignment && (
              <Stack spacing={1.5}>
                {!assignmentPlanMatchesSelection && (
                  <Alert severity="info">
                    This assignment uses its pinned route revisions, which
                    differ from the currently selected profiles. Stop it before
                    creating another assignment.
                  </Alert>
                )}
                <Alert
                  severity={
                    currentAssignment.observedState === "running"
                      ? "success"
                      : currentAssignment.observedState === "failed"
                        ? "error"
                        : "info"
                  }
                >
                  Assignment {currentAssignment.assignment.id} · generation{" "}
                  {currentAssignment.assignment.generation} · worker{" "}
                  {currentAssignment.assignment.workerId}
                  <br />
                  Desired: {currentAssignment.desiredState} · Observed:{" "}
                  {currentAssignment.observedState}
                </Alert>
                {pipelineAssignment.isError && (
                  <Alert severity="error">
                    Assignment refresh failed: {pipelineAssignment.error.message}
                  </Alert>
                )}
                {start.isError &&
                  start.variables?.pipelineId === activePipelineId && (
                    <Alert severity="error">
                      Start request failed: {start.error.message}. The displayed
                      state is refreshed before retrying.
                    </Alert>
                  )}
                {stop.isError &&
                  stop.variables?.pipelineId === activePipelineId && (
                    <Alert severity="error">
                      Stop request failed: {stop.error.message}. Capacity remains
                      reserved until the worker acknowledges Stop.
                    </Alert>
                  )}
                {cancel.isError &&
                  cancel.variables?.pipelineId === activePipelineId && (
                    <Alert severity="error">
                      Cancel request failed: {cancel.error.message}.
                    </Alert>
                  )}
                <Stack direction="row" spacing={1}>
                  {(currentAssignment.desiredState === "planned" ||
                    (currentAssignment.desiredState === "running" &&
                      currentAssignment.observedState === "starting")) && (
                    <Button
                      variant="contained"
                      disabled={lifecycleActionPending}
                      onClick={() =>
                        start.mutate({
                          pipelineId: currentAssignment.assignment.pipelineId,
                          assignmentId: currentAssignment.assignment.id,
                          generation: currentAssignment.assignment.generation,
                        })
                      }
                    >
                      {currentAssignment.observedState === "starting"
                        ? "Retry Start"
                        : "Start"}
                    </Button>
                  )}
                  {(currentAssignment.desiredState === "running" ||
                    (currentAssignment.desiredState === "stopped" &&
                      currentAssignment.observedState === "stopping")) && (
                    <Button
                      variant="outlined"
                      color="warning"
                      disabled={lifecycleActionPending}
                      onClick={() =>
                        stop.mutate({
                          pipelineId: currentAssignment.assignment.pipelineId,
                          assignmentId: currentAssignment.assignment.id,
                          generation: currentAssignment.assignment.generation,
                        })
                      }
                    >
                      {currentAssignment.observedState === "stopping"
                        ? "Retry Stop"
                        : "Stop"}
                    </Button>
                  )}
                  {currentAssignment.desiredState === "planned" &&
                    currentAssignment.observedState === "not_started" && (
                      <Button
                        variant="outlined"
                        color="error"
                        disabled={lifecycleActionPending}
                        onClick={() =>
                          cancel.mutate({
                            pipelineId:
                              currentAssignment.assignment.pipelineId,
                            assignmentId: currentAssignment.assignment.id,
                            generation:
                              currentAssignment.assignment.generation,
                          })
                        }
                      >
                        Cancel unstarted assignment
                      </Button>
                    )}
                </Stack>
              </Stack>
            )}
          </Stack>
        </CardContent>
      </Card>
    </Stack>
  );
}

type ResourceWorkspaceProps =
  | {
      kind: "input";
      loading: boolean;
      error: string;
      records: SourceRecord[];
      saveError: string;
      editingId?: string;
      onEdit: (record: SourceRecord | DestinationRecord) => void;
      onCancelEdit: () => void;
      validationError: string;
      validatingId?: string;
      onValidate: (id: string) => void;
      onSubmit: (event: FormEvent<HTMLFormElement>) => void;
      name: string;
      onNameChange: (value: string) => void;
      profileKind: SourceMediaProfile["kind"];
      onProfileKindChange: (kind: SourceMediaProfile["kind"]) => void;
      sourceDevice: string;
      onSourceDeviceChange: (value: string) => void;
      sourceHost: string;
      onSourceHostChange: (value: string) => void;
      sourceBindAddress: string;
      onSourceBindAddressChange: (value: string) => void;
      sourcePort: string;
      onSourcePortChange: (value: string) => void;
      sourcePayloadType: string;
      onSourcePayloadTypeChange: (value: string) => void;
      sourceMode: "caller" | "listener";
      onSourceModeChange: (value: "caller" | "listener") => void;
      saving: boolean;
    }
  | {
      kind: "output";
      loading: boolean;
      error: string;
      records: DestinationRecord[];
      saveError: string;
      editingId?: string;
      onEdit: (record: SourceRecord | DestinationRecord) => void;
      onCancelEdit: () => void;
      validationError: string;
      validatingId?: string;
      onValidate: (id: string) => void;
      onSubmit: (event: FormEvent<HTMLFormElement>) => void;
      name: string;
      onNameChange: (value: string) => void;
      profileKind: DestinationMediaProfile["kind"];
      onProfileKindChange: (kind: DestinationMediaProfile["kind"]) => void;
      destinationAddress: string;
      onDestinationAddressChange: (value: string) => void;
      destinationPort: string;
      onDestinationPortChange: (value: string) => void;
      destinationPayloadType: string;
      onDestinationPayloadTypeChange: (value: string) => void;
      destinationMode: "caller" | "listener";
      onDestinationModeChange: (value: "caller" | "listener") => void;
      saving: boolean;
    };

function ResourceWorkspace(props: ResourceWorkspaceProps) {
  const isInput = props.kind === "input";
  const label = isInput ? "Input" : "Output";
  const actionLabel = props.editingId
    ? `Save ${label.toLowerCase()} changes`
    : `Create ${label.toLowerCase()} draft`;
  return (
    <Stack spacing={2.5}>
      <Alert severity="info">
        {props.editingId
          ? "Saving creates a new disabled draft revision. Validate it before use; the previous revision is preserved."
          : "New resources are saved as disabled drafts. Validation checks the saved configuration but does not start media. Starting an assignment enables its input and output; they stay enabled while an active assignment uses them and are disabled after the last such assignment stops."}
      </Alert>
      <Box
        sx={{
          display: "grid",
          gridTemplateColumns: { xs: "1fr", lg: "minmax(320px, .8fr) 1.2fr" },
          gap: 2.5,
          alignItems: "start",
        }}
      >
        <Card>
          <CardContent>
            <Stack component="form" spacing={2} onSubmit={props.onSubmit}>
              <Typography variant="h6" component="h2">
                {props.editingId
                  ? `Edit ${label.toLowerCase()}`
                  : `Configure ${label.toLowerCase()}`}
              </Typography>
              <TextField
                label="Friendly name"
                value={props.name}
                onChange={(event) => props.onNameChange(event.target.value)}
                required
                inputProps={{ maxLength: 120 }}
              />
              {isInput ? (
                <SourceFields {...props} />
              ) : (
                <DestinationFields {...props} />
              )}
              {props.saveError && (
                <Alert severity="error">
                  {label} could not be saved: {props.saveError}
                </Alert>
              )}
              <Stack direction="row" spacing={1}>
                <Button
                  type="submit"
                  variant="contained"
                  disabled={props.saving}
                >
                  {props.saving ? "Saving..." : actionLabel}
                </Button>
                {props.editingId && (
                  <Button
                    type="button"
                    disabled={props.saving}
                    onClick={props.onCancelEdit}
                  >
                    Cancel
                  </Button>
                )}
              </Stack>
            </Stack>
          </CardContent>
        </Card>

        <Card>
          <CardContent>
            <Stack spacing={2}>
              <Stack
                direction="row"
                justifyContent="space-between"
                alignItems="center"
              >
                <Typography variant="h6" component="h2">
                  Configured {isInput ? "inputs" : "outputs"}
                </Typography>
                <Chip size="small" label={props.records.length} />
              </Stack>
              {props.error && (
                <Alert severity="error">
                  {label}s could not be loaded: {props.error}
                </Alert>
              )}
              {props.validationError && (
                <Alert severity="error">
                  Validation failed: {props.validationError}
                </Alert>
              )}
              {props.loading ? (
                <Alert severity="info">Loading {label.toLowerCase()}s...</Alert>
              ) : props.records.length === 0 ? (
                <Paper variant="outlined" sx={{ p: 3, textAlign: "center" }}>
                  <Typography color="text.secondary">
                    No {label.toLowerCase()}s configured yet.
                  </Typography>
                </Paper>
              ) : (
                <Stack spacing={1}>
                  {props.records.map((record) => (
                    <ResourceRow
                      key={record.id}
                      record={record}
                      kind={props.kind}
                      validating={props.validatingId === record.id}
                      onValidate={() => props.onValidate(record.id)}
                      onEdit={() => props.onEdit(record)}
                    />
                  ))}
                </Stack>
              )}
            </Stack>
          </CardContent>
        </Card>
      </Box>
    </Stack>
  );
}

function SourceFields(
  props: Extract<ResourceWorkspaceProps, { kind: "input" }>,
) {
  return (
    <>
      <TextField
        select
        label="Input profile"
        value={props.profileKind}
        onChange={(event) =>
          props.onProfileKindChange(
            event.target.value as SourceMediaProfile["kind"],
          )
        }
      >
        <MenuItem value="srt_mpeg_ts">SRT MPEG-TS</MenuItem>
        <MenuItem value="rtp_l16">RTP L16</MenuItem>
        <MenuItem value="rtp_l24">RTP L24</MenuItem>
        <MenuItem value="alsa_l16">ALSA</MenuItem>
        {props.profileKind === "rtp_mp2t" && (
          <MenuItem value="rtp_mp2t">Legacy RTP MPEG-TS</MenuItem>
        )}
      </TextField>
      {(props.profileKind === "rtp_l16" ||
        props.profileKind === "rtp_l24") && (
        <Typography variant="caption" color="text.secondary">
          48 kHz stereo linear PCM · dynamic RTP payload type
        </Typography>
      )}
      {props.profileKind === "alsa_l16" ? (
        <TextField
          label="ALSA device"
          value={props.sourceDevice}
          onChange={(event) => props.onSourceDeviceChange(event.target.value)}
          required
        />
      ) : (
        <>
          <TextField
            label={
              props.profileKind === "srt_mpeg_ts" ? "SRT host" : "Bind address"
            }
            value={
              props.profileKind === "srt_mpeg_ts"
                ? props.sourceHost
                : props.sourceBindAddress
            }
            onChange={(event) => {
              if (props.profileKind === "srt_mpeg_ts") {
                props.onSourceHostChange(event.target.value);
              } else {
                props.onSourceBindAddressChange(event.target.value);
              }
            }}
            required
          />
          <TextField
            label={props.profileKind === "srt_mpeg_ts" ? "SRT port" : "UDP port"}
            type="number"
            value={props.sourcePort}
            onChange={(event) => props.onSourcePortChange(event.target.value)}
            inputProps={{ min: 1, max: 65535 }}
            required
          />
          {(props.profileKind === "rtp_l16" ||
            props.profileKind === "rtp_l24") && (
            <TextField
              label="RTP payload type"
              type="number"
              value={props.sourcePayloadType}
              onChange={(event) =>
                props.onSourcePayloadTypeChange(event.target.value)
              }
              inputProps={{ min: 96, max: 127 }}
              required
            />
          )}
          {props.profileKind === "srt_mpeg_ts" && (
            <TextField
              select
              label="SRT mode"
              value={props.sourceMode}
              onChange={(event) =>
                props.onSourceModeChange(
                  event.target.value as "caller" | "listener",
                )
              }
            >
              <MenuItem value="caller">Caller</MenuItem>
              <MenuItem value="listener">Listener</MenuItem>
            </TextField>
          )}
        </>
      )}
    </>
  );
}

function DestinationFields(
  props: Extract<ResourceWorkspaceProps, { kind: "output" }>,
) {
  return (
    <>
      <TextField
        select
        label="Output profile"
        value={props.profileKind}
        onChange={(event) =>
          props.onProfileKindChange(
            event.target.value as DestinationMediaProfile["kind"],
          )
        }
      >
        <MenuItem value="srt_mpeg_ts">SRT MPEG-TS</MenuItem>
        <MenuItem value="rtp_l16">RTP L16</MenuItem>
        <MenuItem value="rtp_l24">RTP L24</MenuItem>
      </TextField>
      {props.profileKind === "rtp_l24" && (
        <Typography variant="caption" color="text.secondary">
          48 kHz stereo linear PCM · dynamic RTP payload type
        </Typography>
      )}
      <TextField
        label={props.profileKind === "srt_mpeg_ts" ? "Host" : "Address"}
        value={props.destinationAddress}
        onChange={(event) =>
          props.onDestinationAddressChange(event.target.value)
        }
        required
      />
      <TextField
        label="Destination port"
        type="number"
        value={props.destinationPort}
        onChange={(event) => props.onDestinationPortChange(event.target.value)}
        inputProps={{ min: 1, max: 65535 }}
        required
      />
      {props.profileKind !== "srt_mpeg_ts" ? (
        <TextField
          label="RTP payload type"
          type="number"
          value={props.destinationPayloadType}
          onChange={(event) =>
            props.onDestinationPayloadTypeChange(event.target.value)
          }
          inputProps={{ min: 96, max: 127 }}
          required
        />
      ) : (
        <TextField
          select
          label="SRT mode"
          value={props.destinationMode}
          onChange={(event) =>
            props.onDestinationModeChange(
              event.target.value as "caller" | "listener",
            )
          }
        >
          <MenuItem value="caller">Caller</MenuItem>
          <MenuItem value="listener">Listener</MenuItem>
        </TextField>
      )}
    </>
  );
}

function ResourceRow({
  record,
  kind,
  validating,
  onValidate,
  onEdit,
}: {
  record: SourceRecord | DestinationRecord;
  kind: "input" | "output";
  validating: boolean;
  onValidate: () => void;
  onEdit: () => void;
}) {
  return (
    <Paper
      variant="outlined"
      sx={{
        px: 1.5,
        py: 1.25,
        display: "flex",
        justifyContent: "space-between",
        alignItems: "center",
        gap: 1.5,
      }}
    >
      <Box sx={{ minWidth: 0 }}>
        <Typography variant="body2" sx={{ fontWeight: 600 }} noWrap>
          {record.friendlyName}
        </Typography>
        <Typography variant="caption" color="text.secondary">
          {record.configuration.kind} · Revision {record.currentRevision}
        </Typography>
        <Stack direction="row" spacing={0.75} sx={{ mt: 0.75 }}>
          <Chip
            size="small"
            label={record.desiredState === "disabled" ? "Disabled" : "Enabled"}
            variant="outlined"
          />
          <Chip
            size="small"
            label={
              record.lifecycle === "draft"
                ? "Draft"
                : `${record.lifecycle} · ${record.desiredState}`
            }
            color={record.lifecycle === "draft" ? "default" : "success"}
            variant="outlined"
          />
        </Stack>
      </Box>
      <Stack direction="row" spacing={0.5} flexShrink={0}>
        {record.desiredState === "disabled" && (
          <Button
            size="small"
            onClick={onEdit}
            aria-label={`Edit ${kind} ${record.friendlyName}`}
          >
            Edit
          </Button>
        )}
        {record.lifecycle === "draft" && (
          <Button
            size="small"
            variant="outlined"
            disabled={validating}
            onClick={onValidate}
            aria-label={`Validate ${kind} ${record.friendlyName}`}
          >
            {validating ? "Validating..." : "Validate"}
          </Button>
        )}
      </Stack>
    </Paper>
  );
}

function PlannedWorkspace({ title }: { title: string }) {
  return (
    <Paper variant="outlined" sx={{ p: { xs: 3, md: 5 }, textAlign: "center" }}>
      <Stack spacing={1.5} alignItems="center">
        <Chip label="Planned" color="warning" variant="outlined" />
        <Typography variant="h6">{title} is not available yet</Typography>
        <Typography color="text.secondary" sx={{ maxWidth: 560 }}>
          This workspace is reserved for the next implementation phase. No
          controls are shown until GPIO discovery, configuration, and runtime
          behavior are supported.
        </Typography>
      </Stack>
    </Paper>
  );
}

function AdminWorkspace({
  themeMode,
  onThemeChange,
  healthLabel,
  healthy,
  error,
}: {
  themeMode: "dark" | "light";
  onThemeChange: (mode: "dark" | "light") => void;
  healthLabel: string;
  healthy: boolean;
  error: string;
}) {
  return (
    <Box
      sx={{
        display: "grid",
        gridTemplateColumns: { xs: "1fr", md: "minmax(280px, 1fr) 1fr" },
        gap: 2.5,
      }}
    >
      <Card>
        <CardContent>
          <Stack spacing={2}>
            <Typography variant="h6" component="h2">
              Appearance
            </Typography>
            <Typography variant="body2" color="text.secondary">
              Choose the workstation color theme.
            </Typography>
            <FormControlLabel
              control={
                <Switch
                  checked={themeMode === "dark"}
                  onChange={(event) =>
                    onThemeChange(event.target.checked ? "dark" : "light")
                  }
                  inputProps={{ "aria-label": "Use dark theme" }}
                />
              }
              label={`${themeMode === "dark" ? "Dark" : "Light"} theme`}
            />
          </Stack>
        </CardContent>
      </Card>
      <Card>
        <CardContent>
          <Stack spacing={2}>
            <Typography variant="h6" component="h2">
              Service status
            </Typography>
            <Chip
              label={healthLabel}
              color={healthy ? "success" : error ? "error" : "default"}
              variant="outlined"
              sx={{ alignSelf: "flex-start" }}
            />
            {error && <Alert severity="error">{error}</Alert>}
            <Typography variant="body2" color="text.secondary">
              Authentication and operator accounts are not configured in this
              local development build.
            </Typography>
            <Typography variant="body2" color="text.secondary">
              Media runtime and route activation are not available yet.
            </Typography>
          </Stack>
        </CardContent>
      </Card>
    </Box>
  );
}
