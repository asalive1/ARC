import { useState, type FormEvent } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Alert,
  AppBar,
  Box,
  Button,
  Card,
  CardContent,
  Chip,
  Container,
  MenuItem,
  Stack,
  TextField,
  Toolbar,
  Typography,
} from "@mui/material";

import {
  createDestination,
  createSource,
  fetchDestinations,
  fetchLiveness,
  fetchSources,
  planRoute,
  type DestinationMediaProfile,
  type SourceMediaProfile,
} from "./api";

const apiBaseUrl = import.meta.env.VITE_ARC_API_BASE_URL ?? "";

export function App() {
  const queryClient = useQueryClient();
  const [sourceName, setSourceName] = useState("");
  const [sourceKind, setSourceKind] =
    useState<SourceMediaProfile["kind"]>("rtp_mp2t");
  const [sourceDevice, setSourceDevice] = useState("hw:1,0");
  const [sourceBindAddress, setSourceBindAddress] = useState("0.0.0.0");
  const [sourcePort, setSourcePort] = useState("5000");
  const [destinationName, setDestinationName] = useState("");
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
  const sourceMutation = useMutation({
    mutationFn: (input: {
      friendlyName: string;
      profile: SourceMediaProfile;
    }) => createSource(input.friendlyName, input.profile, apiBaseUrl),
    onSuccess: async (created) => {
      setSourceId(created.id);
      await queryClient.invalidateQueries({ queryKey: ["sources"] });
    },
  });
  const destinationMutation = useMutation({
    mutationFn: (input: {
      friendlyName: string;
      profile: DestinationMediaProfile;
    }) => createDestination(input.friendlyName, input.profile, apiBaseUrl),
    onSuccess: async (created) => {
      setDestinationId(created.id);
      await queryClient.invalidateQueries({ queryKey: ["destinations"] });
    },
  });
  const routeMutation = useMutation({
    mutationFn: ({
      source,
      destination,
    }: {
      source: SourceMediaProfile;
      destination: DestinationMediaProfile;
    }) => planRoute(source, destination, apiBaseUrl),
  });

  const selectedSource = sources.data?.find((source) => source.id === sourceId);
  const selectedDestination = destinations.data?.find(
    (destination) => destination.id === destinationId,
  );

  function submitSource(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const profile: SourceMediaProfile =
      sourceKind === "alsa_l16"
        ? { kind: "alsa_l16", device: sourceDevice }
        : {
            kind: "rtp_mp2t",
            bindAddress: sourceBindAddress,
            port: Number(sourcePort),
          };
    sourceMutation.mutate({ friendlyName: sourceName, profile });
  }

  function submitDestination(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const profile: DestinationMediaProfile =
      destinationKind === "rtp_l16"
        ? {
            kind: "rtp_l16",
            address: destinationAddress,
            port: Number(destinationPort),
            payloadType: Number(destinationPayloadType),
          }
        : {
            kind: "srt_mpeg_ts",
            host: destinationAddress,
            port: Number(destinationPort),
            mode: destinationMode,
          };
    destinationMutation.mutate({ friendlyName: destinationName, profile });
  }

  function submitRoute(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!selectedSource || !selectedDestination) {
      return;
    }
    routeMutation.mutate({
      source: selectedSource.configuration,
      destination: selectedDestination.configuration,
    });
  }

  return (
    <Box sx={{ minHeight: "100vh", bgcolor: "grey.100" }}>
      <AppBar position="static">
        <Toolbar>
          <Typography variant="h6" component="h1">
            ARC
          </Typography>
          <Typography sx={{ ml: 2 }} variant="body2">
            Audio Routing Controller
          </Typography>
        </Toolbar>
      </AppBar>
      <Container maxWidth="md" sx={{ py: 5 }}>
        <Stack spacing={3}>
          <Stack direction="row" spacing={1} alignItems="center">
            <Typography variant="h4" component="h2">
              Media configuration
            </Typography>
            <Chip label="Phase 1 preview" size="small" />
          </Stack>
          <Typography color="text.secondary">
            Configure disabled source and destination drafts, then check whether
            their media profiles can be connected. Planning does not start a
            pipeline.
          </Typography>

          <Card>
            <CardContent>
              <Typography variant="h6" component="h3" gutterBottom>
                API liveness
              </Typography>
              {health.isPending ? (
                <Alert severity="info">Checking ARC API...</Alert>
              ) : health.isError ? (
                <Alert severity="error">
                  API unavailable: {health.error.message}
                </Alert>
              ) : (
                <Alert severity="success">ARC API is reachable.</Alert>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardContent>
              <Stack component="form" spacing={2} onSubmit={submitSource}>
                <Typography variant="h6" component="h3">
                  Add source
                </Typography>
                <TextField
                  label="Friendly name"
                  value={sourceName}
                  onChange={(event) => setSourceName(event.target.value)}
                  required
                  inputProps={{ maxLength: 120 }}
                />
                <TextField
                  select
                  label="Source profile"
                  value={sourceKind}
                  onChange={(event) =>
                    setSourceKind(event.target.value as SourceMediaProfile["kind"])
                  }
                >
                  <MenuItem value="rtp_mp2t">RTP MPEG-TS</MenuItem>
                  <MenuItem value="alsa_l16">ALSA L16</MenuItem>
                </TextField>
                {sourceKind === "alsa_l16" ? (
                  <TextField
                    label="ALSA device"
                    value={sourceDevice}
                    onChange={(event) => setSourceDevice(event.target.value)}
                    required
                  />
                ) : (
                  <>
                    <TextField
                      label="Bind address"
                      value={sourceBindAddress}
                      onChange={(event) =>
                        setSourceBindAddress(event.target.value)
                      }
                      required
                    />
                    <TextField
                      label="UDP port"
                      type="number"
                      value={sourcePort}
                      onChange={(event) => setSourcePort(event.target.value)}
                      inputProps={{ min: 1, max: 65535 }}
                      required
                    />
                  </>
                )}
                {sourceMutation.isError && (
                  <Alert severity="error">
                    Source could not be created: {sourceMutation.error.message}
                  </Alert>
                )}
                <Button
                  type="submit"
                  variant="contained"
                  disabled={sourceMutation.isPending}
                >
                  Create source draft
                </Button>
              </Stack>
            </CardContent>
          </Card>

          <Card>
            <CardContent>
              <Stack
                component="form"
                spacing={2}
                onSubmit={submitDestination}
              >
                <Typography variant="h6" component="h3">
                  Add destination
                </Typography>
                <TextField
                  label="Friendly name"
                  value={destinationName}
                  onChange={(event) => setDestinationName(event.target.value)}
                  required
                  inputProps={{ maxLength: 120 }}
                />
                <TextField
                  select
                  label="Destination profile"
                  value={destinationKind}
                  onChange={(event) =>
                    setDestinationKind(
                      event.target.value as DestinationMediaProfile["kind"],
                    )
                  }
                >
                  <MenuItem value="srt_mpeg_ts">SRT MPEG-TS</MenuItem>
                  <MenuItem value="rtp_l16">RTP L16</MenuItem>
                </TextField>
                <TextField
                  label={destinationKind === "rtp_l16" ? "Address" : "Host"}
                  value={destinationAddress}
                  onChange={(event) =>
                    setDestinationAddress(event.target.value)
                  }
                  required
                />
                <TextField
                  label="Destination port"
                  type="number"
                  value={destinationPort}
                  onChange={(event) => setDestinationPort(event.target.value)}
                  inputProps={{ min: 1, max: 65535 }}
                  required
                />
                {destinationKind === "rtp_l16" ? (
                  <TextField
                    label="RTP payload type"
                    type="number"
                    value={destinationPayloadType}
                    onChange={(event) =>
                      setDestinationPayloadType(event.target.value)
                    }
                    inputProps={{ min: 96, max: 127 }}
                    required
                  />
                ) : (
                  <TextField
                    select
                    label="SRT mode"
                    value={destinationMode}
                    onChange={(event) =>
                      setDestinationMode(
                        event.target.value as "caller" | "listener",
                      )
                    }
                  >
                    <MenuItem value="caller">Caller</MenuItem>
                    <MenuItem value="listener">Listener</MenuItem>
                  </TextField>
                )}
                {destinationMutation.isError && (
                  <Alert severity="error">
                    Destination could not be created:{" "}
                    {destinationMutation.error.message}
                  </Alert>
                )}
                <Button
                  type="submit"
                  variant="contained"
                  disabled={destinationMutation.isPending}
                >
                  Create destination draft
                </Button>
              </Stack>
            </CardContent>
          </Card>

          <Card>
            <CardContent>
              <Stack component="form" spacing={2} onSubmit={submitRoute}>
                <Typography variant="h6" component="h3">
                  Check route compatibility
                </Typography>
                {sources.isError && (
                  <Alert severity="error">
                    Sources could not be loaded: {sources.error.message}
                  </Alert>
                )}
                {destinations.isError && (
                  <Alert severity="error">
                    Destinations could not be loaded:{" "}
                    {destinations.error.message}
                  </Alert>
                )}
                <TextField
                  select
                  label="Source"
                  value={sourceId}
                  onChange={(event) => {
                    setSourceId(event.target.value);
                    routeMutation.reset();
                  }}
                  disabled={sources.isLoading || !sources.data?.length}
                  required
                >
                  {(sources.data ?? []).map((source) => (
                    <MenuItem key={source.id} value={source.id}>
                      {source.friendlyName} ({source.configuration.kind})
                    </MenuItem>
                  ))}
                </TextField>
                <TextField
                  select
                  label="Destination"
                  value={destinationId}
                  onChange={(event) => {
                    setDestinationId(event.target.value);
                    routeMutation.reset();
                  }}
                  disabled={
                    destinations.isLoading || !destinations.data?.length
                  }
                  required
                >
                  {(destinations.data ?? []).map((destination) => (
                    <MenuItem key={destination.id} value={destination.id}>
                      {destination.friendlyName} (
                      {destination.configuration.kind})
                    </MenuItem>
                  ))}
                </TextField>
                {routeMutation.isError && (
                  <Alert severity="error">
                    Route is not compatible: {routeMutation.error.message}
                  </Alert>
                )}
                {routeMutation.isSuccess && (
                  <Alert severity="success">
                    Compatible media path: {routeMutation.data.mediaPath}
                  </Alert>
                )}
                <Button
                  type="submit"
                  variant="contained"
                  disabled={
                    routeMutation.isPending ||
                    !selectedSource ||
                    !selectedDestination
                  }
                >
                  Check compatibility
                </Button>
              </Stack>
            </CardContent>
          </Card>

          <Stack spacing={1}>
            <Typography variant="h6" component="h3">
              Configured resources
            </Typography>
            {sources.isLoading || destinations.isLoading ? (
              <Alert severity="info">Loading catalog...</Alert>
            ) : (
              <>
                {(sources.data ?? []).map((source) => (
                  <Alert
                    key={source.id}
                    severity="info"
                    icon={false}
                    action={<Chip label={source.lifecycle} size="small" />}
                  >
                    Source: {source.friendlyName} · {source.configuration.kind}
                  </Alert>
                ))}
                {(destinations.data ?? []).map((destination) => (
                  <Alert
                    key={destination.id}
                    severity="info"
                    icon={false}
                    action={<Chip label={destination.lifecycle} size="small" />}
                  >
                    Destination: {destination.friendlyName} ·{" "}
                    {destination.configuration.kind}
                  </Alert>
                ))}
              </>
            )}
          </Stack>
        </Stack>
      </Container>
    </Box>
  );
}
