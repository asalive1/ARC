import { useEffect, useMemo, useState, type FormEvent } from "react";
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
  Checkbox,
  CircularProgress,
  CssBaseline,
  Divider,
  Dialog,
  DialogActions,
  DialogContent,
  DialogContentText,
  DialogTitle,
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

import { CrosspointMatrix } from "./CrosspointMatrix";
import { stageRouteSelection, type StagedRoute } from "./routeStaging";
import {
  ApiRequestError,
  bootstrapAdmin,
  changeAdminPassword,
  cancelAssignment as cancelAssignmentRequest,
  cancelCapacityRequest,
  fetchAdminAuthStatus,
  fetchAdminSession,
  createDestination as createDestinationRequest,
  createSource as createSourceRequest,
  fetchDestinations,
  fetchReadiness,
  fetchPipelineAssignment,
  fetchCapacityRequest,
  fetchRoutes,
  fetchSources,
  getRouteBatchOperation,
  loginAdmin,
  logoutAdmin,
  planRoute,
  planRouteAssignment,
  setDesiredRoute,
  setDesiredRoutesBatch,
  startAssignment as startAssignmentRequest,
  stopAssignment as stopAssignmentRequest,
  updateDestination as updateDestinationRequest,
  updateSource as updateSourceRequest,
  recoverAdmin,
  setCsrfToken,
  validateDestination,
  validateSource,
  type DestinationMediaProfile,
  type DestinationRecord,
  type PlanRouteAssignmentInput,
  type RouteRecord,
  type RoutePlan,
  type SetDesiredRouteInput,
  type SourceMediaProfile,
  type SourceRecord,
} from "./api";

const apiBaseUrl = import.meta.env.VITE_ARC_API_BASE_URL ?? "";
const capacityAttemptStorageKey = "arc-capacity-attempt";

function storedCapacityAttempt(): PlanRouteAssignmentInput | undefined {
  try {
    const raw = sessionStorage.getItem(capacityAttemptStorageKey);
    if (!raw) return undefined;
    const value = JSON.parse(raw) as PlanRouteAssignmentInput;
    if (value.allowWaiting === true && typeof value.pipelineId === "string" &&
        typeof value.idempotencyKey === "string" && typeof value.sourceId === "string" &&
        typeof value.sourceRevision === "number" && typeof value.destinationId === "string" &&
        typeof value.destinationRevision === "number" &&
        (value.capacityRequestId === undefined || typeof value.capacityRequestId === "string")) {
      return value;
    }
  } catch {
    // Storage can be unavailable or contain an obsolete value.
  }
  return undefined;
}

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
  const [recoveryToken, setRecoveryToken] = useState<string>();
  const [logoutError, setLogoutError] = useState("");
  const authStatus = useQuery({
    queryKey: ["auth", "status"],
    queryFn: () => fetchAdminAuthStatus(apiBaseUrl),
    retry: false,
  });
  const session = useQuery({
    queryKey: ["auth", "session"],
    queryFn: () => fetchAdminSession(apiBaseUrl),
    enabled: authStatus.isSuccess && !authStatus.data.bootstrap_required,
    retry: false,
  });

  useEffect(() => {
    const refreshSession = () => {
      setCsrfToken(undefined);
      void queryClient.invalidateQueries({ queryKey: ["auth", "session"] });
    };
    window.addEventListener("arc-auth-required", refreshSession);
    return () => window.removeEventListener("arc-auth-required", refreshSession);
  }, [queryClient]);

  useEffect(() => {
    setCsrfToken(session.data?.csrf_token);
  }, [session.data?.csrf_token]);

  if (authStatus.isPending) {
    return <AuthLoading />;
  }
  if (authStatus.isError) {
    return (
      <AuthMessage
        message="Administrator authentication could not be reached."
        onRetry={() => void authStatus.refetch()}
      />
    );
  }
  if (authStatus.data.bootstrap_required) {
    return (
      <AdminAuthScreen
        mode="bootstrap"
        onAuthenticated={(csrf) => {
          setCsrfToken(csrf);
          void queryClient.invalidateQueries({ queryKey: ["auth", "status"] });
          void queryClient.invalidateQueries({ queryKey: ["auth", "session"] });
        }}
        onRecovered={() => undefined}
      />
    );
  }
  if (session.isPending) {
    return <AuthLoading />;
  }
  if (session.isError) {
    if (
      session.error instanceof ApiRequestError &&
      session.error.status !== 401
    ) {
      return (
        <AuthMessage
          message="The administrator session could not be verified."
          onRetry={() => void session.refetch()}
        />
      );
    }
    return (
      <AdminAuthScreen
        mode="login"
        onAuthenticated={(csrf) => {
          setCsrfToken(csrf);
          void queryClient.invalidateQueries({ queryKey: ["auth", "session"] });
        }}
        onRecovered={(token, csrf) => {
          setRecoveryToken(token);
          setCsrfToken(csrf);
          void queryClient.invalidateQueries({ queryKey: ["auth", "status"] });
          void queryClient.invalidateQueries({ queryKey: ["auth", "session"] });
        }}
      />
    );
  }

  async function handleLogout() {
    setLogoutError("");
    try {
      await logoutAdmin(apiBaseUrl);
      setCsrfToken(undefined);
      await queryClient.invalidateQueries({ queryKey: ["auth", "session"] });
    } catch (error) {
      setLogoutError(
        error instanceof Error ? error.message : "Unable to sign out.",
      );
    }
  }

  return (
    <WorkspaceApp
      username={session.data.username}
      logoutError={logoutError}
      recoveryToken={recoveryToken}
      onLogout={handleLogout}
      onDismissRecovery={() => setRecoveryToken(undefined)}
      onPasswordChanged={() => {
        setCsrfToken(undefined);
        void queryClient.invalidateQueries({ queryKey: ["auth", "session"] });
      }}
    />
  );
}

function AuthLoading() {
  return (
    <Box
      sx={{
        minHeight: "100vh",
        display: "grid",
        placeItems: "center",
        bgcolor: "#0d151c",
        color: "white",
      }}
    >
      <CircularProgress aria-label="Loading administrator authentication" />
    </Box>
  );
}

function AuthMessage({
  message,
  onRetry,
}: {
  message: string;
  onRetry: () => void;
}) {
  return (
    <Box sx={{ maxWidth: 520, mx: "auto", mt: 10, p: 3 }}>
      <Stack spacing={2}>
        <Alert severity="error">{message}</Alert>
        <Button onClick={onRetry} variant="contained">
          Retry
        </Button>
      </Stack>
    </Box>
  );
}

function AdminAuthScreen({
  mode: initialMode,
  onAuthenticated,
  onRecovered,
}: {
  mode: "bootstrap" | "login";
  onAuthenticated: (csrfToken: string) => void;
  onRecovered: (recoveryToken: string, csrfToken: string) => void;
}) {
  const [mode, setMode] = useState<"bootstrap" | "login" | "recover">(
    initialMode,
  );
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [secretToken, setSecretToken] = useState("");
  const [error, setError] = useState("");
  const [pending, setPending] = useState(false);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setError("");
    if (mode !== "login" && password !== confirmPassword) {
      setError("The password entries do not match.");
      return;
    }
    setPending(true);
    try {
      if (mode === "bootstrap") {
        const result = await bootstrapAdmin(
          secretToken,
          username,
          password,
          apiBaseUrl,
        );
        onAuthenticated(result.csrf_token);
      } else if (mode === "login") {
        const result = await loginAdmin(username, password, apiBaseUrl);
        onAuthenticated(result.csrf_token);
      } else {
        const result = await recoverAdmin(
          secretToken,
          password,
          apiBaseUrl,
        );
        onRecovered(result.replacement_recovery_token, result.csrf_token);
      }
    } catch (requestError) {
      setError(
        requestError instanceof ApiRequestError && requestError.status === 401
          ? "The supplied credentials were rejected."
          : "The request failed. Check the values and try again.",
      );
    } finally {
      setPending(false);
    }
  }

  const bootstrap = mode === "bootstrap";
  const recovering = mode === "recover";

  return (
    <Box
      sx={{
        minHeight: "100vh",
        display: "grid",
        placeItems: "center",
        px: 2,
        bgcolor: "#0d151c",
      }}
    >
      <Card sx={{ width: "100%", maxWidth: 440 }}>
        <CardContent sx={{ p: 3 }}>
          <Stack component="form" spacing={2} onSubmit={submit}>
            <Typography variant="h5" component="h1">
              {bootstrap
                ? "Set up ARC administrator"
                : recovering
                  ? "Recover administrator access"
                  : "Sign in to ARC"}
            </Typography>
            <Typography variant="body2" color="text.secondary">
              {bootstrap
                ? "Use the one-time bootstrap secret from your deployment to create the administrator account."
                : recovering
                  ? "Use the recovery secret to reset the administrator password. A replacement recovery secret will be shown once."
                  : "Sign in with the local administrator account."}
            </Typography>
            {error && <Alert severity="error">{error}</Alert>}
            {(bootstrap || recovering) && (
              <TextField
                label={bootstrap ? "Bootstrap secret" : "Recovery secret"}
                value={secretToken}
                onChange={(event) => setSecretToken(event.target.value)}
                autoComplete="off"
                required
              />
            )}
            {(bootstrap || mode === "login") && (
              <TextField
                label="Username"
                value={username}
                onChange={(event) => setUsername(event.target.value)}
                autoComplete="username"
                required
              />
            )}
            <TextField
              label={mode === "login" ? "Password" : "New password"}
              type="password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              autoComplete={mode === "login" ? "current-password" : "new-password"}
              inputProps={{ minLength: mode === "login" ? undefined : 12, maxLength: 128 }}
              required
            />
            {mode !== "login" && (
              <TextField
                label="Confirm new password"
                type="password"
                value={confirmPassword}
                onChange={(event) => setConfirmPassword(event.target.value)}
                autoComplete="new-password"
                inputProps={{ minLength: 12, maxLength: 128 }}
                required
              />
            )}
            <Button type="submit" variant="contained" disabled={pending}>
              {pending
                ? "Please wait..."
                : bootstrap
                  ? "Create administrator"
                  : recovering
                    ? "Reset password"
                    : "Sign in"}
            </Button>
            {!bootstrap && (
              <Button
                onClick={() => {
                  setError("");
                  setMode(recovering ? "login" : "recover");
                  setPassword("");
                  setConfirmPassword("");
                  setSecretToken("");
                }}
              >
                {recovering ? "Return to sign in" : "Recover access"}
              </Button>
            )}
          </Stack>
        </CardContent>
      </Card>
    </Box>
  );
}

function WorkspaceApp({
  username,
  logoutError,
  recoveryToken,
  onLogout,
  onDismissRecovery,
  onPasswordChanged,
}: {
  username: string;
  logoutError: string;
  recoveryToken?: string;
  onLogout: () => Promise<void>;
  onDismissRecovery: () => void;
  onPasswordChanged: () => void;
}) {
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
  const [assignmentRequest, setAssignmentRequestState] =
    useState<PlanRouteAssignmentInput | undefined>(storedCapacityAttempt);
  const setAssignmentRequest = (value: PlanRouteAssignmentInput | undefined) => {
    try {
      if (value?.allowWaiting) {
        sessionStorage.setItem(capacityAttemptStorageKey, JSON.stringify(value));
      } else {
        sessionStorage.removeItem(capacityAttemptStorageKey);
      }
    } catch {
      // The request remains usable in this tab when storage is unavailable.
    }
    setAssignmentRequestState(value);
  };

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
    queryKey: ["api", "readiness"],
    queryFn: () => fetchReadiness(apiBaseUrl),
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
  const routes = useQuery({
    queryKey: ["routes"],
    queryFn: () => fetchRoutes(apiBaseUrl),
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
            width: { xs: 220, md: 250 },
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
            sx={{ minHeight: 88, px: 2, borderBottom: 1, borderColor: "divider" }}
          >
            <Box
              component="img"
              src={
                themeMode === "dark"
                  ? "/ARC-Light-128.png"
                  : "/ARC-Dark-128.png"
              }
              alt=""
              aria-hidden="true"
              sx={{ width: 64, height: 64, objectFit: "contain", flexShrink: 0 }}
            />
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
            {navigation.map((item) => (
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
          <Stack spacing={0.75} sx={{ p: 2 }}>
            <Typography variant="body2" sx={{ fontWeight: 600 }}>
              {username}
            </Typography>
            <Typography variant="caption" color="text.secondary">
              Administrator
            </Typography>
            <Button size="small" onClick={() => void onLogout()}>
              Sign out
            </Button>
          </Stack>
        </Box>

        <Box sx={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column" }}>
          <Box
            component="header"
            sx={{
              minHeight: 88,
              px: { xs: 2, md: 3 },
              display: "flex",
              alignItems: "center",
              justifyContent: "flex-end",
              gap: 2,
              borderBottom: 1,
              borderColor: "divider",
              bgcolor: "background.paper",
            }}
          >
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
              {logoutError && <Alert severity="error">{logoutError}</Alert>}
              {recoveryToken && (
                <Alert
                  severity="warning"
                  action={
                    <Button color="inherit" size="small" onClick={onDismissRecovery}>
                      I saved it
                    </Button>
                  }
                >
                  Save this replacement administrator recovery token securely.
                  It will not be shown again: <strong>{recoveryToken}</strong>
                </Alert>
              )}
              <Stack
                direction={{ xs: "column", md: "row" }}
                alignItems={{ xs: "flex-start", md: "center" }}
                justifyContent="space-between"
                spacing={1}
              >
                <Box>
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
                  routes={routes.data ?? []}
                  routesLoading={routes.isLoading}
                  routesError={routes.isError ? routes.error.message : ""}
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
                  onPasswordChanged={onPasswordChanged}
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
  routes,
  routesLoading,
  routesError,
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
  routes: RouteRecord[];
  routesLoading: boolean;
  routesError: string;
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
  const capacityRequestId = assignmentRequest?.capacityRequestId;
  const [allowWaiting, setAllowWaiting] = useState(!!assignmentRequest?.allowWaiting);
  const [routeMode, setRouteMode] = useState<"immediate" | "take">("immediate");
  const [pendingTake, setPendingTake] = useState<StagedRoute[]>([]);
  const [takeIdempotencyKey, setTakeIdempotencyKey] = useState<string>();
  const [confirmTakeReplacements, setConfirmTakeReplacements] = useState(false);
  const [routeBatchOrigin, setRouteBatchOrigin] = useState<"immediate" | "take">(
    "take",
  );
  const resultMatchesSelection =
    selectedSource !== undefined &&
    selectedDestination !== undefined &&
    routePlan.variables !== undefined &&
    JSON.stringify(routePlan.variables.source) === JSON.stringify(selectedSource.configuration) &&
    JSON.stringify(routePlan.variables.destination) === JSON.stringify(selectedDestination.configuration);
  const [replaceRoute, setReplaceRoute] = useState<{
    input: SetDesiredRouteInput;
    sourceName: string;
    destinationName: string;
    replacedSourceName: string;
    action: "save" | "execute";
  } | null>(null);
  const destinationRoute = routes.find(
    (route) => route.destinationId === selectedDestination?.id,
  );
  const routeRequiresReplacement = (
    currentRoute: RouteRecord | undefined,
    sourceId: string,
  ) =>
    currentRoute !== undefined &&
    ((currentRoute.desiredState === "desired" &&
      currentRoute.sourceId !== sourceId) ||
      (currentRoute.observedState !== "stopped" &&
        currentRoute.observedSourceId !== undefined &&
        currentRoute.observedSourceId !== sourceId));
  const pendingTakeReplacements = pendingTake.filter((route) =>
    routeRequiresReplacement(
      routes.find((item) => item.destinationId === route.destinationId),
      route.sourceId,
    ),
  );
  const pendingTakeHasStaleRevision = pendingTake.some((route) => {
    const currentRevision =
      routes.find((item) => item.destinationId === route.destinationId)
        ?.revision ?? 0;
    return currentRevision !== route.expectedRouteRevision;
  });
  const routeReplacementRequired =
    selectedSource !== undefined &&
    routeRequiresReplacement(destinationRoute, selectedSource.id);
  const routeAlreadyRunning =
    destinationRoute?.observedState === "running" &&
    destinationRoute.observedSourceId === selectedSource?.id;
  const routeExecutionBlockReason =
    destinationRoute &&
    destinationRoute.desiredState !== "desired" &&
    destinationRoute.observedState !== "stopped"
      ? "This destination has route history that is still transitioning. Wait for it to stop before starting another source."
      : undefined;
  const routeWrite = useMutation({
    mutationFn: (input: SetDesiredRouteInput) =>
      setDesiredRoute(input, apiBaseUrl),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["routes"] });
    },
  });
  const routeBatch = useMutation({
    mutationFn: (input: {
      routes: SetDesiredRouteInput[];
      idempotencyKey: string;
    }) =>
      setDesiredRoutesBatch(
        input.routes,
        input.idempotencyKey,
        apiBaseUrl,
      ),
    onSuccess: async () => {
      setPendingTake([]);
      setTakeIdempotencyKey(undefined);
      setConfirmTakeReplacements(false);
      await queryClient.invalidateQueries({ queryKey: ["routes"] });
    },
    onError: async (error) => {
      if (error instanceof ApiRequestError && error.status === 409) {
        await queryClient.invalidateQueries({ queryKey: ["routes"] });
      }
    },
  });
  const routeBatchStatus = useQuery({
    queryKey: ["route-batch-operation", routeBatch.data?.operationId],
    queryFn: () =>
      getRouteBatchOperation(routeBatch.data!.operationId, apiBaseUrl),
    enabled: routeBatch.data?.operationId !== undefined,
    refetchInterval: (query) =>
      query.state.data?.state === "pending" ||
      query.state.data?.state === "running"
        ? 1000
        : false,
    retry: false,
  });
  const routeBatchExecutionPending =
    routeBatchStatus.data?.state === "pending" ||
    routeBatchStatus.data?.state === "running" ||
    routeBatch.data?.state === "pending" ||
    routeBatch.data?.state === "running";
  useEffect(() => {
    const state = routeBatchStatus.data?.state;
    if (state === "completed" || state === "partial" || state === "failed") {
      void queryClient.invalidateQueries({ queryKey: ["routes"] });
    }
  }, [queryClient, routeBatchStatus.data?.state]);
  const assignmentPlan = useMutation({
    mutationFn: (input: PlanRouteAssignmentInput) =>
      planRouteAssignment(input, apiBaseUrl),
    onSuccess: (result, input) => {
      if (result.requestId) {
        queryClient.setQueryData(["capacity-request", result.requestId], result);
        onAssignmentRequestChange({ ...input, capacityRequestId: result.requestId });
      } else {
        onAssignmentRequestChange(result.assignment ? input : undefined);
      }
    },
  });
  const capacityStatus = useQuery({
    queryKey: ["capacity-request", capacityRequestId],
    queryFn: () => fetchCapacityRequest(capacityRequestId!, apiBaseUrl),
    enabled: capacityRequestId !== undefined,
    refetchInterval: (query) =>
      !query.state.data || query.state.data.state === "waiting" ? 1000 : false,
    retry: false,
  });
  const cancelWait = useMutation({
    mutationFn: (id: string) => cancelCapacityRequest(id, apiBaseUrl),
    onSuccess: (result, id) => {
      queryClient.setQueryData(["capacity-request", id], result);
    },
    onError: (_error, id) => {
      void queryClient.invalidateQueries({ queryKey: ["capacity-request", id] });
    },
  });
  const waitResult = capacityRequestId ? capacityStatus.data : undefined;
  const waitPending = !!assignmentRequest?.allowWaiting &&
    (!capacityRequestId || !waitResult || waitResult.state === "waiting");
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
      (!assignmentRequest?.allowWaiting || waitResult?.state === "planned") &&
      (assignmentPlan.isIdle ||
        assignmentPlan.isError ||
        assignmentPlan.data?.assignment != null ||
        waitResult?.state === "planned"),
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
  const lifecycleActionPending =
    start.isPending || stop.isPending || cancel.isPending;
  const executeRoute = useMutation({
    mutationFn: async (input: {
      source: SourceRecord;
      destination: DestinationRecord;
    }) => {
      const existingRoute = routes.find(
        (route) => route.destinationId === input.destination.id,
      );
      if (
        !existingRoute ||
        existingRoute.desiredState !== "desired" ||
        existingRoute.sourceId !== input.source.id
      ) {
        await routeWrite.mutateAsync({
          sourceId: input.source.id,
          destinationId: input.destination.id,
          expectedRouteRevision: existingRoute?.revision ?? 0,
          confirmReplace: false,
        });
      }

      const assignmentInput: PlanRouteAssignmentInput = {
        pipelineId: createPipelineId(),
        idempotencyKey: createPipelineId(),
        sourceId: input.source.id,
        sourceRevision: input.source.currentRevision,
        destinationId: input.destination.id,
        destinationRevision: input.destination.currentRevision,
      };
      const planResult = await assignmentPlan
        .mutateAsync(assignmentInput)
        .catch((error: unknown) => {
          const message =
            error instanceof Error ? error.message : "unknown planning error";
          throw new Error(
            `Desired route is set, but worker planning failed: ${message}`,
          );
        });
      if (planResult.state !== "planned" || !planResult.assignment) {
        throw new Error(
          `Desired route is set, but no worker assignment was created${
            planResult.reason ? `: ${planResult.reason}` : "."
          }`,
        );
      }

      const assignment = planResult.assignment;
      const snapshot = await start
        .mutateAsync({
          pipelineId: assignment.pipelineId,
          assignmentId: assignment.id,
          generation: assignment.generation,
        })
        .catch((error: unknown) => {
          const message =
            error instanceof Error ? error.message : "unknown worker error";
          throw new Error(
            `Desired route is set and planned, but worker Start was not confirmed: ${message}`,
          );
        });
      return { snapshot, pipelineId: assignment.pipelineId };
    },
    onSuccess: async (result) => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["routes"] }),
        queryClient.invalidateQueries({
          queryKey: ["pipeline-assignment", result.pipelineId],
        }),
      ]);
      routeWrite.reset();
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
  const assignmentPlanResult = capacityRequestId
    ? waitResult
    : assignmentPlanMatchesSelection ? assignmentPlan.data : undefined;
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
    assignmentPlanResult?.state !== "unavailable" &&
    assignmentPlanResult?.state !== "cancelled";
  const canCreateAssignment =
    routeMode === "immediate" &&
    selectedSource?.lifecycle === "validated" &&
    selectedDestination?.lifecycle === "validated" &&
    !assignmentPlan.isPending &&
    !waitPending &&
    (activePipelineId === undefined ||
      assignmentPlanResult?.state === "unavailable" ||
      assignmentPlanResult?.state === "cancelled" ||
      !pipelineAssignment.isFetching) &&
    !assignmentStatusUnknown &&
    (!currentAssignment || currentAssignmentIsTerminal) &&
    (assignmentPlanResult?.assignment == null ||
      currentAssignmentIsTerminal);
  const canExecuteSelectedRoute =
    selectedSource?.lifecycle === "validated" &&
    selectedDestination?.lifecycle === "validated" &&
    !routesLoading &&
    !routeWrite.isPending &&
    !routeBatch.isPending &&
    !routeBatchExecutionPending &&
    !assignmentPlan.isPending &&
    !waitPending &&
    !lifecycleActionPending &&
    !executeRoute.isPending &&
    !routeExecutionBlockReason &&
    !routeAlreadyRunning &&
    (routeReplacementRequired ||
      (canCreateAssignment &&
        (!destinationRoute ||
          (destinationRoute.desiredState === "desired" &&
            destinationRoute.sourceId === selectedSource.id)) &&
        destinationRoute?.observedState !== "running" &&
        (!destinationRoute?.observedSourceId ||
          destinationRoute.observedSourceId === selectedSource.id)));
  const canSaveTake =
    routeMode === "take" &&
    pendingTake.length > 0 &&
    !routesLoading &&
    !pendingTakeHasStaleRevision &&
    pendingTake.every((route) => {
      const source = sources.find((item) => item.id === route.sourceId);
      const destination = destinations.find(
        (item) => item.id === route.destinationId,
      );
      return source?.lifecycle === "validated" &&
        destination?.lifecycle === "validated";
    }) &&
    (pendingTakeReplacements.length === 0 || confirmTakeReplacements) &&
    !routeBatch.isPending &&
    !routeBatchExecutionPending &&
    !routeWrite.isPending;
  const routeBatchResult = routeBatchStatus.data ?? routeBatch.data;
  const executeRouteMatchesSelection =
    executeRoute.variables?.source.id === selectedSource?.id &&
    executeRoute.variables?.destination.id === selectedDestination?.id;

  async function invalidateResourceRecords() {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ["sources"] }),
      queryClient.invalidateQueries({ queryKey: ["destinations"] }),
      queryClient.invalidateQueries({ queryKey: ["routes"] }),
    ]);
  }

  function submitDesiredRoute() {
    if (
      routeMode !== "immediate" ||
      !selectedSource ||
      !selectedDestination ||
      !routePlan.isSuccess ||
      !resultMatchesSelection ||
      selectedSource.lifecycle !== "validated" ||
      selectedDestination.lifecycle !== "validated"
    ) {
      return;
    }

    routeWrite.reset();
    executeRoute.reset();
    const input: SetDesiredRouteInput = {
      sourceId: selectedSource.id,
      destinationId: selectedDestination.id,
      expectedRouteRevision: destinationRoute?.revision ?? 0,
      confirmReplace: false,
    };
    if (
      destinationRoute?.desiredState === "desired" &&
      destinationRoute.sourceId !== selectedSource.id
    ) {
      const replacedSource = sources.find(
        (source) => source.id === destinationRoute.sourceId,
      );
      setReplaceRoute({
        input: { ...input, confirmReplace: true },
        sourceName: selectedSource.friendlyName,
        destinationName: selectedDestination.friendlyName,
        replacedSourceName: replacedSource?.friendlyName ?? "another input",
        action: "save",
      });
      return;
    }
    routeWrite.mutate(input);
  }

  function submitTake() {
    if (!canSaveTake) return;
    const idempotencyKey = takeIdempotencyKey ?? createPipelineId();
    setTakeIdempotencyKey(idempotencyKey);
    setRouteBatchOrigin("take");
    routeBatch.mutate({
      idempotencyKey,
      routes: pendingTake.map((route) => {
        const currentRoute = routes.find(
          (item) => item.destinationId === route.destinationId,
        );
        return {
          sourceId: route.sourceId,
          destinationId: route.destinationId,
          expectedRouteRevision: route.expectedRouteRevision,
          confirmReplace: routeRequiresReplacement(currentRoute, route.sourceId),
        };
      }),
    });
  }

  function submitImmediateRoute() {
    if (
      routeMode !== "immediate" ||
      !selectedSource ||
      !selectedDestination ||
      !canExecuteSelectedRoute
    ) {
      return;
    }

    routeWrite.reset();
    if (routeReplacementRequired) {
      const replacedSource = sources.find(
        (source) =>
          source.id ===
          (destinationRoute?.observedSourceId ?? destinationRoute?.sourceId),
      );
      setReplaceRoute({
        input: {
          sourceId: selectedSource.id,
          destinationId: selectedDestination.id,
          expectedRouteRevision: destinationRoute?.revision ?? 0,
          confirmReplace: true,
        },
        sourceName: selectedSource.friendlyName,
        destinationName: selectedDestination.friendlyName,
        replacedSourceName: replacedSource?.friendlyName ?? "the current input",
        action: "execute",
      });
      return;
    }
    executeRoute.mutate({
      source: selectedSource,
      destination: selectedDestination,
    });
  }

  function submitAssignment() {
    if (
      routeMode !== "immediate" ||
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
      !capacityRequestId &&
      assignmentPlanResult?.state !== "unavailable" &&
      assignmentPlanResult?.state !== "cancelled" &&
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
    const input: PlanRouteAssignmentInput = {
      pipelineId,
      idempotencyKey,
      sourceId: selectedSource.id,
      sourceRevision: selectedSource.currentRevision,
      destinationId: selectedDestination.id,
      destinationRevision: selectedDestination.currentRevision,
      ...(retryingSamePlan && previousAttempt?.allowWaiting
        ? { allowWaiting: true }
        : allowWaiting ? { allowWaiting: true } : {}),
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
              Select an input/output crosspoint. Immediate mode starts an
              eligible route; Take mode stages selections for review.
            </Typography>
          </Box>
          <Chip label="Up to 64 × 64" size="small" />
        </Stack>
        <CrosspointMatrix
          sources={sources}
          destinations={destinations}
          routes={routes}
          routesLoading={routesLoading}
          routesError={routesError}
          selectedSourceId={selectedSourceId}
          selectedDestinationId={selectedDestinationId}
          onSelect={(sourceId, destinationId) => {
            onSourceChange(sourceId);
            onDestinationChange(destinationId);
            if (routeMode === "take") {
              const staged = stageRouteSelection(pendingTake, {
                sourceId,
                destinationId,
                expectedRouteRevision:
                  routes.find((route) => route.destinationId === destinationId)
                    ?.revision ?? 0,
              });
              if (staged !== pendingTake) {
                setPendingTake(staged);
                setTakeIdempotencyKey(createPipelineId());
                setConfirmTakeReplacements(false);
                routeBatch.reset();
              }
            }
          }}
        />
        <FormControlLabel
          control={
            <Switch
              checked={routeMode === "take"}
              disabled={pendingTake.length > 0}
              onChange={(_, checked) =>
                setRouteMode(checked ? "take" : "immediate")
              }
            />
          }
          label={
            routeMode === "take"
              ? "Take mode (stage changes)"
              : "Immediate mode"
          }
        />
        {routeMode === "take" && (
          <Stack spacing={1.5} sx={{ mt: 2 }}>
            <Alert severity="info">
              A Take saves all desired routes atomically, then executes each
              route independently. Confirmed replacements prepare the new path
              before stopping the current route; failed preparation leaves the
              current route untouched. After the hard cut, ARC attempts to
              restore the previous route if the replacement cannot start.
              Outcomes appear below; one route does not roll back successful
              sibling routes.
            </Alert>
            {pendingTakeHasStaleRevision && (
              <Alert severity="warning">
                A destination changed after staging. Select its crosspoint
                again to refresh the expected revision, or cancel the Take.
              </Alert>
            )}
            {pendingTake.length === 0 ? (
              <Typography variant="body2" color="text.secondary">
                Select crosspoints in the matrix to add them to this Take.
              </Typography>
            ) : (
              <Stack spacing={1}>
                <Typography variant="subtitle2">
                  Pending Take ({pendingTake.length})
                </Typography>
                {pendingTake.map((route) => {
                  const source = sources.find(
                    (item) => item.id === route.sourceId,
                  );
                  const destination = destinations.find(
                    (item) => item.id === route.destinationId,
                  );
                  return (
                    <Paper
                      key={route.destinationId}
                      variant="outlined"
                      sx={{ p: 1.25 }}
                    >
                      <Stack
                        direction="row"
                        alignItems="center"
                        justifyContent="space-between"
                        spacing={1}
                      >
                        <Typography variant="body2">
                          {source?.friendlyName ?? "Unavailable input"} →{" "}
                          {destination?.friendlyName ?? "Unavailable output"}
                        </Typography>
                        <Button
                          size="small"
                          onClick={() => {
                            setPendingTake((current) =>
                              current.filter(
                                (item) =>
                                  item.destinationId !== route.destinationId,
                              ),
                            );
                            setTakeIdempotencyKey(createPipelineId());
                            setConfirmTakeReplacements(false);
                            routeBatch.reset();
                          }}
                        >
                          Remove
                        </Button>
                      </Stack>
                    </Paper>
                  );
                })}
              </Stack>
            )}
            {pendingTakeReplacements.length > 0 && (
              <Stack spacing={1}>
                <Alert severity="warning">
                  This Take replaces the desired source for{" "}
                  {pendingTakeReplacements.length} destination
                  {pendingTakeReplacements.length === 1 ? "" : "s"}. Any
                  active route will continue while ARC prepares its
                  replacement. Switching is a hard cut, not a seamless
                  crossfade; if the new route fails to start, ARC attempts to
                  restore the previous route.
                </Alert>
                <FormControlLabel
                  control={
                    <Checkbox
                      checked={confirmTakeReplacements}
                      onChange={(_, checked) =>
                        setConfirmTakeReplacements(checked)
                      }
                    />
                  }
                  label="Confirm replacing existing desired routes"
                />
              </Stack>
            )}
            {routeBatch.isError && (
              <Alert severity="error">
                {routeBatch.error instanceof ApiRequestError &&
                routeBatch.error.status === 409
                  ? "The batch conflicted with current route state. No changes were saved; refresh and re-stage the Take."
                  : "The Take could not be confirmed. Retry it to safely reuse its operation key before staging a new Take."}
              </Alert>
            )}
            {routeBatchStatus.isError && (
              <Alert severity="error">
                The Take was accepted, but its execution status could not be
                refreshed. Operation {routeBatch.data?.operationId} remains
                available for review.
              </Alert>
            )}
            {routeBatch.isSuccess && routeBatchResult && (
              <Alert
                severity={
                  routeBatchResult.state === "completed"
                    ? "success"
                    : routeBatchResult.state === "partial"
                      ? "warning"
                      : routeBatchResult.state === "failed"
                        ? "error"
                        : "info"
                }
              >
                {routeBatchResult.state === "completed"
                  ? `Take completed; ${routeBatchResult.results?.length ?? routeBatchResult.routes.length} route(s) were acknowledged by workers.`
                  : routeBatchResult.state === "partial"
                    ? "Take partially completed. Review the per-route outcomes below."
                    : routeBatchResult.state === "failed"
                      ? "Take failed. Review the per-route outcomes below."
                      : routeBatchResult.state === "desired_state_applied"
                        ? "Desired routes were saved, but this API has not reported worker execution."
                        : routeBatchResult.state === "running"
                          ? "Take execution is in progress. Waiting for worker acknowledgements."
                          : "Take was durably accepted and is waiting for worker execution."}{" "}
                (operation {routeBatchResult.operationId}).
              </Alert>
            )}
            {(routeBatchResult?.results ?? []).map((result) => {
              const source = sources.find((item) => item.id === result.sourceId);
              const destination = destinations.find(
                (item) => item.id === result.destinationId,
              );
              return (
                <Alert
                  key={result.destinationId}
                  severity={
                    result.state === "succeeded"
                      ? "success"
                      : result.state === "failed"
                        ? "error"
                        : "info"
                  }
                >
                  {source?.friendlyName ?? "Unavailable input"} →{" "}
                  {destination?.friendlyName ?? "Unavailable output"}:{" "}
                  {result.state === "succeeded"
                    ? "worker acknowledged Start"
                    : result.state === "failed"
                      ? `${result.errorCode ?? "execution_failed"} — ${result.errorMessage ?? "No further details are available."}`
                      : result.errorMessage ??
                        (result.state === "processing"
                          ? "starting on a worker"
                          : "waiting for a worker")}
                </Alert>
              );
            })}
            <Stack direction={{ xs: "column", sm: "row" }} spacing={1}>
              <Button
                variant="outlined"
                disabled={pendingTake.length === 0}
                onClick={() => {
                  setPendingTake([]);
                  setTakeIdempotencyKey(undefined);
                  setConfirmTakeReplacements(false);
                  routeBatch.reset();
                }}
              >
                Cancel Take
              </Button>
              <Button
                variant="contained"
                disabled={!canSaveTake}
                onClick={submitTake}
              >
                {routeBatch.isPending ? "Submitting Take…" : "Execute Take"}
              </Button>
            </Stack>
          </Stack>
        )}
        <Stack
          spacing={1.5}
          sx={{
            mt: 2,
            display: routeMode === "immediate" ? undefined : "none",
          }}
        >
          <Typography variant="body2" color="text.secondary">
            Start selected route assigns a worker and requests Start. For a
            confirmed replacement, the new path is prepared before the current
            route is stopped. Switching is a hard cut; if the new route fails
            to start, ARC attempts to restore the previous route.
          </Typography>
          <Button
            variant="contained"
            onClick={submitImmediateRoute}
            disabled={!canExecuteSelectedRoute}
          >
            {routeBatch.isPending
              ? "Submitting replacement…"
              : routeReplacementRequired
                ? "Replace selected route"
                : executeRoute.isPending
                  ? "Starting route…"
                  : "Start selected route"}
          </Button>
          {routeExecutionBlockReason && (
            <Alert severity="info">{routeExecutionBlockReason}</Alert>
          )}
          {executeRoute.isPending && executeRouteMatchesSelection && (
            <Alert severity="info">
              Desired route is set. Planning a worker assignment and waiting
              for its Start response; media is not confirmed yet.
            </Alert>
          )}
          {executeRoute.isSuccess &&
            executeRouteMatchesSelection &&
            (executeRoute.data.snapshot.observedState === "running" ? (
              <Alert severity="success">
                Worker confirmed the route as running. The matrix is
                refreshing observed state.
              </Alert>
            ) : (
              <Alert severity="info">
                Start returned worker state "
                {executeRoute.data.snapshot.observedState}". The matrix is
                refreshing observed state.
              </Alert>
            ))}
          {executeRoute.isError &&
            executeRouteMatchesSelection && (
              <Alert severity="error">
                Unable to start route: {executeRoute.error.message}
              </Alert>
            )}
          {routeBatchOrigin === "immediate" && routeBatch.isError && (
            <Alert severity="error">
              {routeBatch.error instanceof ApiRequestError &&
              routeBatch.error.status === 409
                ? "The route changed before replacement. Refresh the matrix and try again."
                : `The replacement could not be submitted: ${routeBatch.error.message}`}
            </Alert>
          )}
          {routeBatchOrigin === "immediate" &&
            routeBatchStatus.isError && (
              <Alert severity="error">
                The replacement was accepted, but its status could not be
                refreshed. Operation {routeBatch.data?.operationId} remains
                available for review.
              </Alert>
            )}
          {routeBatchOrigin === "immediate" &&
            routeBatch.isSuccess &&
            routeBatchResult && (
              <>
                <Alert
                  severity={
                    routeBatchResult.state === "completed"
                      ? "success"
                      : routeBatchResult.state === "failed" ||
                          routeBatchResult.state === "partial"
                        ? "warning"
                        : "info"
                  }
                >
                  {routeBatchResult.state === "completed"
                    ? "The replacement was acknowledged by the worker."
                    : routeBatchResult.state === "failed"
                      ? "The replacement failed. Review the recovery result below."
                      : routeBatchResult.state === "running"
                        ? "The replacement is in progress; waiting for worker acknowledgements."
                        : "The replacement was accepted and is waiting for execution."}{" "}
                  (operation {routeBatchResult.operationId}).
                </Alert>
                {(routeBatchResult.results ?? []).map((result) => (
                  <Alert
                    key={result.destinationId}
                    severity={result.state === "succeeded" ? "success" : result.state === "failed" ? "warning" : "info"}
                  >
                    {sources.find((item) => item.id === result.sourceId)?.friendlyName ??
                      "Unavailable input"}{" "}
                    →{" "}
                    {destinations.find(
                      (item) => item.id === result.destinationId,
                    )?.friendlyName ?? "Unavailable output"}
                    :{" "}
                    {result.state === "succeeded"
                      ? "worker acknowledged Start"
                      : result.errorMessage ??
                        (result.state === "processing"
                          ? "executing on a worker"
                          : "waiting for a worker")}
                  </Alert>
                ))}
              </>
            )}
          <Button
            variant="outlined"
            onClick={submitDesiredRoute}
            disabled={
              !selectedSource ||
              !selectedDestination ||
              selectedSource.lifecycle !== "validated" ||
              selectedDestination.lifecycle !== "validated" ||
              !routePlan.isSuccess ||
              !resultMatchesSelection ||
              routesLoading ||
              routeWrite.isPending ||
              (destinationRoute?.desiredState === "desired" &&
                destinationRoute.sourceId === selectedSource.id)
            }
          >
            {routeWrite.isPending
              ? "Saving desired route…"
              : destinationRoute?.desiredState === "desired" &&
                  destinationRoute.sourceId === selectedSource?.id
                ? "Route already desired"
                : "Save desired route"}
          </Button>
          {routeWrite.isSuccess &&
            !executeRouteMatchesSelection &&
            routeWrite.variables?.sourceId === selectedSource?.id &&
            routeWrite.variables?.destinationId === selectedDestination?.id && (
              <Alert severity="success">
                Desired route saved only. Media has not been started or
                confirmed.
              </Alert>
            )}
          {routeWrite.isError &&
            routeWrite.variables?.sourceId === selectedSource?.id &&
            routeWrite.variables?.destinationId === selectedDestination?.id && (
              <Alert severity="error">
                Unable to save route: {routeWrite.error.message}
              </Alert>
            )}
        </Stack>
        <Dialog
          open={replaceRoute !== null}
          onClose={() => setReplaceRoute(null)}
        >
          <DialogTitle>
            {replaceRoute?.action === "execute"
              ? "Replace active route?"
              : "Replace desired route?"}
          </DialogTitle>
          <DialogContent>
            <DialogContentText>
              {replaceRoute
                ? replaceRoute.action === "execute"
                  ? `Replace ${replaceRoute.replacedSourceName} on ${replaceRoute.destinationName} with ${replaceRoute.sourceName}? ARC will prepare the new path first, then stop the current route and start the replacement. This is a hard cut; if Start fails, ARC will attempt to restore the previous route.`
                  : `Replace the desired route from ${replaceRoute.replacedSourceName} to ${replaceRoute.destinationName} with ${replaceRoute.sourceName}? This saves desired state only; it does not switch live media.`
                : ""}
            </DialogContentText>
          </DialogContent>
          <DialogActions>
            <Button onClick={() => setReplaceRoute(null)}>Cancel</Button>
            <Button
              variant="contained"
              disabled={routeWrite.isPending || routeBatch.isPending}
              onClick={() => {
                if (!replaceRoute) return;
                const { input, action } = replaceRoute;
                setReplaceRoute(null);
                if (action === "save") {
                  routeWrite.mutate(input);
                } else {
                  const idempotencyKey = createPipelineId();
                  setTakeIdempotencyKey(idempotencyKey);
                  setRouteBatchOrigin("immediate");
                  routeBatch.mutate({
                    idempotencyKey,
                    routes: [input],
                  });
                }
              }}
            >
              {replaceRoute?.action === "execute"
                ? "Confirm and execute"
                : "Confirm replacement"}
            </Button>
          </DialogActions>
        </Dialog>
        <Stack
          direction={{ xs: "column", sm: "row" }}
          justifyContent="space-between"
          spacing={1}
          sx={{ mt: 1.5 }}
        >
          <Typography variant="caption" color="text.secondary">
            Selected:{" "}
            {selectedSource?.friendlyName ?? "no input"} →{" "}
            {selectedDestination?.friendlyName ?? "no output"}
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

      <Card sx={{ display: routeMode === "immediate" ? undefined : "none" }}>
        <CardContent>
          <Stack spacing={2}>
            <Box>
              <Typography variant="h6" component="h2">
                Worker assignment
              </Typography>
              <Typography variant="body2" color="text.secondary">
                Reserve capacity for the selected validated input and output,
                then Start or Stop it; unstarted assignments can be cancelled.
                Planning alone does not start media. New plans are rejected
                while that destination has a planned or non-stopped assignment.
                Use Immediate or Take execution to replace an occupied route;
                direct assignment planning does not switch routes.
              </Typography>
            </Box>
            {(selectedSource?.lifecycle !== "validated" ||
              selectedDestination?.lifecycle !== "validated") && (
              <Alert severity="info">
                Validate the selected input and output before assigning a
                worker.
              </Alert>
            )}
            <FormControlLabel
              control={
                <Switch
                  checked={allowWaiting}
                  onChange={(_, checked) => setAllowWaiting(checked)}
                  disabled={waitPending || assignmentPlan.isPending}
                />
              }
              label="Wait up to five minutes for capacity (strict FIFO)"
            />
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
            {assignmentRequest?.allowWaiting && !capacityRequestId &&
              !assignmentPlan.isPending && (
                <Alert severity="warning" action={<Stack direction="row">
                  <Button color="inherit" size="small" onClick={() => assignmentPlan.mutate(assignmentRequest)}>
                    Recover request
                  </Button>
                  {assignmentPlan.error instanceof ApiRequestError &&
                    [400, 404, 409, 422].includes(assignmentPlan.error.status) && (
                      <Button color="inherit" size="small" onClick={() => {
                        onAssignmentRequestChange(undefined);
                        assignmentPlan.reset();
                      }}>Clear rejected request</Button>
                    )}
                </Stack>}>
                  The wait request outcome is unknown. Recover it using the same
                  idempotency key before creating another request.
                </Alert>
              )}
            {capacityRequestId && waitPending && (
              <Alert severity={capacityStatus.isError ? "warning" : "info"} action={
                <Button color="inherit" size="small" disabled={cancelWait.isPending}
                  onClick={() => cancelWait.mutate(capacityRequestId)}>
                  Cancel wait
                </Button>
              }>
                Waiting for capacity, request {capacityRequestId}
                {waitResult?.expiresAt ? `, expires ${new Date(waitResult.expiresAt).toLocaleString()}` : ""}.
                {capacityStatus.isError ? " Status refresh failed; retrying." : " The queue is strict FIFO."}
                {cancelWait.isError ? ` Cancellation failed: ${cancelWait.error.message}` : ""}
              </Alert>
            )}
            {capacityRequestId && capacityStatus.error instanceof ApiRequestError &&
              capacityStatus.error.status === 404 && (
                <Alert severity="warning" action={<Button color="inherit" size="small"
                  onClick={() => {
                    onAssignmentRequestChange(undefined);
                    assignmentPlan.reset();
                  }}>Clear local request</Button>}>
                  This request was not found or has been purged. Clearing it does
                  not create a new request.
                </Alert>
              )}
            {assignmentPlanResult?.state === "cancelled" && (
              <Alert severity="info">Capacity wait cancelled. No reservation was created.</Alert>
            )}
            {assignmentPlan.isError && assignmentPlanMatchesSelection && (
              <Alert
                severity="error"
                action={
                  <Button
                    color="inherit"
                    size="small"
                    onClick={() => assignmentRequest?.allowWaiting
                      ? assignmentPlan.mutate(assignmentRequest)
                      : void pipelineAssignment.refetch()}
                  >
                    Check status
                  </Button>
                }
              >
                Assignment request failed: {assignmentPlan.error.message}.{" "}
                {assignmentRequest?.allowWaiting
                ? "Recover the wait request using its original key."
                : currentAssignment
                  ? "A persisted assignment was found and loaded."
                  : "The same pipeline ID is being checked for a committed assignment."}
                {pipelineAssignment.isError
                  ? ` Lookup failed: ${pipelineAssignment.error.message}`
                  : ""}
              </Alert>
            )}
            {assignmentPlanResult?.state === "unavailable" && (
              <Alert severity="warning">
                {assignmentPlanResult.reason === "expired"
                  ? "Capacity wait expired. It will not retry automatically."
                  : `No eligible worker could be assigned.${assignmentPlanResult.reason ? ` Reason: ${assignmentPlanResult.reason}.` : ""}`}
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
              !waitPending &&
              assignmentPlanResult?.state !== "unavailable" &&
              assignmentPlanResult?.state !== "cancelled" &&
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
  onPasswordChanged,
}: {
  themeMode: "dark" | "light";
  onThemeChange: (mode: "dark" | "light") => void;
  healthLabel: string;
  healthy: boolean;
  error: string;
  onPasswordChanged: () => void;
}) {
  const [currentPassword, setCurrentPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [passwordError, setPasswordError] = useState("");
  const [changingPassword, setChangingPassword] = useState(false);

  async function changePassword(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setPasswordError("");
    if (newPassword !== confirmPassword) {
      setPasswordError("The password entries do not match.");
      return;
    }
    setChangingPassword(true);
    try {
      await changeAdminPassword(currentPassword, newPassword, apiBaseUrl);
      onPasswordChanged();
    } catch (requestError) {
      setPasswordError(
        requestError instanceof ApiRequestError && requestError.status === 401
          ? "The current password was rejected."
          : "The password could not be changed. Check the values and try again.",
      );
    } finally {
      setChangingPassword(false);
    }
  }

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
              Local administrator authentication is enabled. Worker credentials
              are independently provisioned through deployment secrets.
            </Typography>
            <Typography variant="body2" color="text.secondary">
              Media runtime and route activation are not available yet.
            </Typography>
          </Stack>
        </CardContent>
      </Card>
      <Card>
        <CardContent>
          <Stack component="form" spacing={2} onSubmit={changePassword}>
            <Typography variant="h6" component="h2">
              Administrator password
            </Typography>
            <Typography variant="body2" color="text.secondary">
              Changing the password revokes all active administrator sessions.
              Sign in again after the update.
            </Typography>
            {passwordError && <Alert severity="error">{passwordError}</Alert>}
            <TextField
              label="Current password"
              type="password"
              value={currentPassword}
              onChange={(event) => setCurrentPassword(event.target.value)}
              autoComplete="current-password"
              required
            />
            <TextField
              label="New password"
              type="password"
              value={newPassword}
              onChange={(event) => setNewPassword(event.target.value)}
              autoComplete="new-password"
              inputProps={{ minLength: 12, maxLength: 128 }}
              required
            />
            <TextField
              label="Confirm new password"
              type="password"
              value={confirmPassword}
              onChange={(event) => setConfirmPassword(event.target.value)}
              autoComplete="new-password"
              inputProps={{ minLength: 12, maxLength: 128 }}
              required
            />
            <Button type="submit" variant="contained" disabled={changingPassword}>
              {changingPassword ? "Changing..." : "Change password"}
            </Button>
          </Stack>
        </CardContent>
      </Card>
    </Box>
  );
}
