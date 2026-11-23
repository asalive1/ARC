import { useQuery } from "@tanstack/react-query";
import {
  Alert,
  AppBar,
  Box,
  Card,
  CardContent,
  Chip,
  Container,
  Stack,
  Toolbar,
  Typography,
} from "@mui/material";

import { fetchLiveness } from "./api";

const apiBaseUrl = import.meta.env.VITE_ARC_API_BASE_URL ?? "";

export function App() {
  const health = useQuery({
    queryKey: ["api", "liveness"],
    queryFn: () => fetchLiveness(apiBaseUrl),
    retry: 1,
  });

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
              Control plane
            </Typography>
            <Chip label="Phase 0" size="small" />
          </Stack>
          <Typography color="text.secondary">
            Workspace foundation is in place. Source, destination, routing, and
            media controls will arrive in later vertical slices.
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
        </Stack>
      </Container>
    </Box>
  );
}
