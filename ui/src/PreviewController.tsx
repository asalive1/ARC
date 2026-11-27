import { useEffect, useRef, useState } from "react";
import {
  Alert, Button, Paper, Stack, Typography,
} from "@mui/material";

import { PreviewMeters } from "./PreviewMeters";
import {
  connectPcmPreview, type PcmPreviewPlayer, type PcmPreviewStream, type StereoLevels,
} from "./pcmPreview";

const silent: StereoLevels = { left: -60, right: -60 };

type PreviewConnection = { player: PcmPreviewPlayer; stream: PcmPreviewStream };
type PreviewRun = {
  abort: AbortController;
  opening: Promise<PreviewConnection>;
  connection?: PreviewConnection;
};
type PreviewState = {
  name: string;
  phase: "idle" | "connecting" | "playing" | "stopping" | "error";
  error?: string;
  levels: StereoLevels;
};

function previewUrl(baseUrl: string, sourceId: string): string {
  const url = new URL(`${baseUrl.replace(/\/$/, "")}/api/v1/sources/${encodeURIComponent(sourceId)}/preview`, window.location.href);
  if (url.origin !== window.location.origin) throw new Error("Audio preview requires the same API origin.");
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  return url.toString();
}

export function usePreviewController(baseUrl: string) {
  const run = useRef<PreviewRun | undefined>(undefined);
  const [state, setState] = useState<PreviewState>({ name: "", phase: "idle", levels: silent });

  function start(sourceId: string, name: string) {
    if (run.current || state.phase === "stopping") return;
    let url: string;
    try {
      url = previewUrl(baseUrl, sourceId);
    } catch (error) {
      setState({ name, phase: "error", error: error instanceof Error ? error.message : String(error), levels: silent });
      return;
    }
    const abort = new AbortController();
    setState({ name, phase: "connecting", levels: silent });
    let current: PreviewRun;
    const opening = connectPcmPreview(url, abort.signal, (error) => {
      if (run.current !== current) return;
      run.current = undefined;
      setState({ name, phase: "error", error: error.message, levels: silent });
    });
    current = { abort, opening };
    run.current = current;
    void current.opening.then((connection) => {
      if (run.current !== current) return;
      current.connection = connection;
      setState({ name, phase: "playing", levels: silent });
    }, (error: unknown) => {
      if (run.current !== current) return;
      run.current = undefined;
      setState({ name, phase: "error", error: error instanceof Error ? error.message : String(error), levels: silent });
    });
  }

  async function stop() {
    const current = run.current;
    run.current = undefined;
    if (!current) {
      setState({ name: "", phase: "idle", levels: silent });
      return;
    }
    current.abort.abort();
    setState((previous) => ({ ...previous, phase: "stopping", levels: silent }));
    try {
      const connection = current.connection ?? await current.opening;
      await connection.stream.close();
      setState({ name: "", phase: "idle", levels: silent });
    } catch (error) {
      if (error instanceof Error && error.message === "Preview connection cancelled") {
        setState({ name: "", phase: "idle", levels: silent });
      } else {
        setState((previous) => ({
          ...previous, phase: "error",
          error: error instanceof Error ? error.message : String(error), levels: silent,
        }));
      }
    }
  }

  useEffect(() => {
    if (state.phase !== "playing") return;
    const timer = setInterval(() => {
      const current = run.current;
      if (!current?.connection) return;
      try {
        const levels = current.connection.player.levels();
        setState((previous) => ({ ...previous, levels }));
      } catch (error) {
        current.abort.abort();
        run.current = undefined;
        void current.connection.stream.close().then(
          () => setState((previous) => ({
            ...previous, phase: "error",
            error: error instanceof Error ? error.message : String(error), levels: silent,
          })),
          (cleanup: unknown) => setState((previous) => ({
            ...previous, phase: "error",
            error: `Audio preview and cleanup failed: ${String(error)}; ${String(cleanup)}`, levels: silent,
          })),
        );
      }
    }, 100);
    return () => clearInterval(timer);
  }, [state.phase]);

  useEffect(() => () => {
    const current = run.current;
    run.current = undefined;
    if (!current) return;
    current.abort.abort();
    void current.opening.then(
      ({ stream }) => stream.close(),
      (error: unknown) => {
        if (!(error instanceof Error && error.message === "Preview connection cancelled")) {
          console.error("Preview cleanup failed", error);
        }
      },
    ).catch((error: unknown) => console.error("Preview cleanup failed", error));
  }, []);

  const dialog = (
    state.phase !== "idle" && (
      <Paper role="dialog" aria-label={`Audio preview: ${state.name}`} elevation={8}
        sx={{ position: "fixed", right: 24, bottom: 24, width: 360, maxWidth: "calc(100vw - 48px)", zIndex: 1300, p: 2 }}>
        <Stack spacing={1}>
        <Typography variant="h6">Audio preview: {state.name}</Typography>
        {state.phase === "connecting" && <Typography>Connecting to input...</Typography>}
        {state.phase === "stopping" && <Typography>Stopping preview...</Typography>}
        {state.phase === "error" && <Alert severity="error">{state.error}</Alert>}
        <PreviewMeters levels={state.levels} />
        <Button onClick={() => void stop()} disabled={state.phase === "stopping"} sx={{ alignSelf: "flex-end" }}>
          {state.phase === "error" ? "Close" : "Stop"}
        </Button>
        </Stack>
      </Paper>
    )
  );
  return { start, dialog, state };
}
