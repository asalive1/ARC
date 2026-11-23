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
  createDestination as createDestinationRequest,
  createSource as createSourceRequest,
  fetchDestinations,
  fetchLiveness,
  fetchSources,
  planRoute,
  updateDestination as updateDestinationRequest,
  updateSource as updateSourceRequest,
  validateDestination,
  validateSource,
  type DestinationMediaProfile,
  type DestinationRecord,
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
              Local development · Media execution not active
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
              Crosspoint routing will be available when route activation and
              worker execution are implemented.
            </Typography>
          </Box>
          <Chip label="Inactive · no routes applied" size="small" />
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
            {routePlan.isError && (
              <Alert severity="error">
                No compatible path: {routePlan.error.message}
              </Alert>
            )}
            {routePlan.isSuccess && (
              <Alert severity="success">
                Compatible path: {routePlan.data.mediaPath}. This is a plan
                only; no media is flowing.
              </Alert>
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
          : "New resources are saved as disabled drafts. Validation checks the saved configuration; it does not start media or activate a route."}
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
                : `${record.lifecycle} · inactive`
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
