import { useEffect, useRef, useState, type MouseEvent } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Accordion, AccordionDetails, AccordionSummary, Alert, Button,
  Dialog, DialogActions, DialogContent, DialogContentText, DialogTitle,
  Menu, MenuItem, Paper, Stack, TextField, Typography,
} from "@mui/material";

import { CrosspointMatrix } from "./CrosspointMatrix";
import { PreviewMeters } from "./PreviewMeters";
import type { StereoLevels } from "./pcmPreview";
import {
  ApiRequestError, deletePreset, executePreset, fetchPresets,
  getRouteBatchOperation, savePreset, setDesiredRoutesBatch, unrouteDestination,
  type DestinationRecord, type RouteBatchItemResult, type RoutePreset, type RouteRecord,
  type SetDesiredRouteInput, type SourceRecord,
} from "./api";
import {
  createPipelineId, hasStaleRouteSelection, matchesPresetRoutes, snapshotPresetRoutes,
  toggleRouteSelection, type StagedRoute,
} from "./routeStaging";

interface RoutingWorkspaceProps {
  apiBaseUrl: string;
  mode: "routes" | "salvos";
  active?: boolean;
  sources: SourceRecord[];
  destinations: DestinationRecord[];
  routes: RouteRecord[];
  routesLoading: boolean;
  routesError: string;
  sourceError: string;
  destinationError: string;
  onSelect?: (sourceId: string, destinationId: string) => void;
  onPreview?: (sourceId: string) => void;
  preview?: { name: string; levels: StereoLevels; busy: boolean };
}

type RouteAction =
  | { kind: "take"; routes: SetDesiredRouteInput[]; key: string }
  | { kind: "unroute"; destinationId: string; expectedRouteRevision: number; key: string }
  | { kind: "salvo"; presetId: string; expectedRevision: number; routes: SetDesiredRouteInput[]; key: string };

function isExecuting(state: string | undefined): boolean {
  return state === "pending" || state === "running";
}

function requiresReplacement(route: RouteRecord | undefined, sourceId: string): boolean {
  return route !== undefined &&
    ((route.desiredState === "desired" && route.sourceId !== sourceId) ||
      (route.observedState !== "stopped" && route.observedSourceId != null &&
        route.observedSourceId !== sourceId));
}

export function DestinationOutcome({ item, name }: {
  item: RouteBatchItemResult;
  name: string;
}) {
  return (
    <Alert severity={item.state === "succeeded"
      ? "success" : item.state === "failed" ? "error" : "info"}>
      {name}{": "}{item.action === "stop" ? "Unroute" : "Take"}{" - "}{item.state}
      {item.errorMessage ? ` - ${item.errorMessage}` : ""}
      {item.errorCode === "destination_in_use" &&
        " The output may still have a previous or failed assignment. If it is safe to disconnect, use Unroute and wait for confirmation before retrying Take."}
    </Alert>
  );
}

export function RoutingWorkspace({
  apiBaseUrl, mode, active = true, sources, destinations, routes, routesLoading,
  routesError, sourceError, destinationError, onSelect, onPreview, preview,
}: RoutingWorkspaceProps) {
  const queryClient = useQueryClient();
  const draftSession = useRef(0);
  const [selections, setSelections] = useState<StagedRoute[]>([]);
  const [preset, setPreset] = useState<RoutePreset>();
  const [menu, setMenu] = useState<{
    selection: StagedRoute; left: number; top: number;
  } | null>(null);
  const [confirmation, setConfirmation] = useState<RouteAction | null>(null);
  const [saveOpen, setSaveOpen] = useState(false);
  const [saveName, setSaveName] = useState("");
  const [deleteOpen, setDeleteOpen] = useState(false);
  const salvos = mode === "salvos";
  const presets = useQuery({
    queryKey: ["presets"],
    queryFn: () => fetchPresets(apiBaseUrl),
    enabled: salvos,
    retry: false,
  });
  const operation = useMutation({
    mutationFn: (action: RouteAction) => {
      switch (action.kind) {
        case "take":
          return setDesiredRoutesBatch(action.routes, action.key, apiBaseUrl);
        case "unroute":
          return unrouteDestination({
            destinationId: action.destinationId,
            expectedRouteRevision: action.expectedRouteRevision,
          }, action.key, apiBaseUrl);
        case "salvo":
          return executePreset(action.presetId, {
            expectedRevision: action.expectedRevision, routes: action.routes,
          }, action.key, apiBaseUrl);
      }
    },
    onSettled: async (_result, _error, action) => {
      await queryClient.invalidateQueries({ queryKey: ["routes"] });
      if (action.kind === "salvo") {
        await queryClient.invalidateQueries({ queryKey: ["presets"] });
      }
    },
  });
  const status = useQuery({
    queryKey: ["route-batch-operation", operation.data?.operationId],
    queryFn: () => getRouteBatchOperation(operation.data!.operationId, apiBaseUrl),
    enabled: operation.data !== undefined,
    refetchInterval: (query) =>
      !query.state.data || isExecuting(query.state.data.state) ? 1000 : false,
    retry: false,
  });
  const result = status.data ?? operation.data;
  useEffect(() => {
    if (result && !isExecuting(result.state)) {
      void Promise.all([
        queryClient.invalidateQueries({ queryKey: ["routes"] }),
        queryClient.invalidateQueries({ queryKey: ["sources"] }),
        queryClient.invalidateQueries({ queryKey: ["destinations"] }),
      ]);
    }
  }, [queryClient, result?.state, result?.operationId]);
  const unknownOutcome = operation.isError &&
    (!(operation.error instanceof ApiRequestError) || operation.error.status >= 500);
  const busy = operation.isPending || isExecuting(result?.state) ||
    (operation.data !== undefined && status.isError) || unknownOutcome;
  const save = useMutation({
    mutationFn: (input: Parameters<typeof savePreset>[0]) => savePreset(input, apiBaseUrl),
    onMutate: () => ({ session: draftSession.current }),
    onSuccess: async (saved, _input, context) => {
      queryClient.setQueryData<RoutePreset[]>(["presets"], (current) => [
        ...(current ?? []).filter((item) => item.id !== saved.id),
        saved,
      ]);
      if (context?.session === draftSession.current) {
        setPreset(saved);
        setSaveOpen(false);
      }
      await queryClient.invalidateQueries({ queryKey: ["presets"] });
    },
    onError: async (error) => {
      if (error instanceof ApiRequestError && error.status === 409) {
        await queryClient.invalidateQueries({ queryKey: ["presets"] });
      }
    },
  });
  const remove = useMutation({
    mutationFn: (saved: RoutePreset) => deletePreset(saved.id, saved.revision, apiBaseUrl),
    onMutate: () => ({ session: draftSession.current }),
    onSuccess: async (_result, saved, context) => {
      queryClient.setQueryData<RoutePreset[]>(["presets"], (current) =>
        (current ?? []).filter((item) => item.id !== saved.id));
      if (context?.session === draftSession.current) {
        setPreset(undefined);
        setSelections([]);
        setDeleteOpen(false);
      }
      await queryClient.invalidateQueries({ queryKey: ["presets"] });
    },
    onError: async (error) => {
      if (error instanceof ApiRequestError && error.status === 409) {
        await queryClient.invalidateQueries({ queryKey: ["presets"] });
      }
    },
  });
  useEffect(() => {
    if (!salvos) return;
    draftSession.current += 1;
    setSelections([]);
    setPreset(undefined);
    setSaveName("");
    setSaveOpen(false);
    setDeleteOpen(false);
    setConfirmation(null);
    setMenu(null);
    save.reset();
    remove.reset();
  }, [active, salvos, save.reset, remove.reset]);

  const controlsBusy = busy || save.isPending || remove.isPending;
  const routeStateUnavailable = routesLoading || !!routesError || !!sourceError || !!destinationError;
  const stale = hasStaleRouteSelection(selections, routes);
  const dirty = !preset || !matchesPresetRoutes(selections, preset.routes);
  const invalidSelection = selections.some((selection) => {
    const source = sources.find((source) => source.id === selection.sourceId);
    const destination = destinations.find((destination) => destination.id === selection.destinationId);
    return !source || !destination || (selection.action !== "stop" &&
      (source.lifecycle !== "validated" || destination.lifecycle !== "validated"));
  });
  const presetChanged = preset !== undefined && presets.isSuccess &&
    !presets.data.some((saved) => saved.id === preset.id && saved.revision === preset.revision);
  const presetOptions = [...(presets.data ?? [])];
  if (preset && !presetOptions.some((saved) => saved.id === preset.id)) {
    presetOptions.push(preset);
  }

  function choosePreset(id: string) {
    const saved = presets.data?.find((item) => item.id === id);
    setPreset(saved);
    setSelections(saved ? snapshotPresetRoutes(saved.routes, routes) : []);
    save.reset();
    remove.reset();
  }

  function saveDraft(update: boolean) {
    save.mutate({
      name: saveName.trim(),
      routes: selections.map(({ sourceId, destinationId, action }) => ({ sourceId, destinationId, action })),
      ...(update && preset ? { id: preset.id, expectedRevision: preset.revision } : {}),
    });
  }

  function select(sourceId: string, destinationId: string) {
    if (controlsBusy) return;
    const selection = snapshotPresetRoutes([{ sourceId, destinationId }], routes)[0];
    setSelections((current) => salvos
      ? toggleRouteSelection(current, selection) : [selection]);
    onSelect?.(sourceId, destinationId);
  }

  function openMenu(event: MouseEvent<HTMLButtonElement>, sourceId: string, destinationId: string) {
    event.preventDefault();
    if (controlsBusy || salvos) return;
    const selection = snapshotPresetRoutes([{ sourceId, destinationId }], routes)[0];
    setSelections([selection]);
    onSelect?.(sourceId, destinationId);
    setMenu({ selection, left: event.clientX, top: event.clientY });
  }

  function take(changes: StagedRoute[], saved?: RoutePreset) {
    const input = changes.map((selection) => ({
      ...selection,
      confirmReplace: selection.action !== "stop" && requiresReplacement(
        routes.find((route) => route.destinationId === selection.destinationId),
        selection.sourceId,
      ),
    }));
    const action: RouteAction = saved
      ? { kind: "salvo", presetId: saved.id, expectedRevision: saved.revision,
          routes: input, key: createPipelineId() }
      : { kind: "take", routes: input, key: createPipelineId() };
    setMenu(null);
    if (input.some((route) => route.confirmReplace)) {
      setConfirmation(action);
    } else {
      operation.mutate(action);
    }
  }

  const menuRoute = routes.find((route) => route.destinationId === menu?.selection.destinationId);
  const menuCanTake = menu !== null && !routeStateUnavailable &&
    sources.find((source) => source.id === menu.selection.sourceId)?.lifecycle === "validated" &&
    destinations.find((destination) => destination.id === menu.selection.destinationId)?.lifecycle === "validated" &&
    !hasStaleRouteSelection([menu.selection], routes);
  const menuCanUnroute = menu !== null && !routeStateUnavailable &&
    !hasStaleRouteSelection([menu.selection], routes) && menuRoute !== undefined &&
    (menuRoute.desiredState === "desired" || menuRoute.observedState !== "stopped");

  return (
    <Paper variant="outlined" sx={{ p: { xs: 2, md: 3 } }}>
      <Stack spacing={2}>
        <Typography variant="body2" color="text.secondary">
          {salvos
            ? "Click a crosspoint to cycle: blue to activate, red to unroute its output, then clear. Red disconnects the output regardless of its input. Save and name the selections, then Take the saved salvo. Returning to this screen starts a new empty draft."
            : "Left-click to highlight a crosspoint. Right-click it for Take, Unroute, or Preview input. Take starts that route; Unroute disconnects that output."}
        </Typography>
        {!salvos && preview && (
          <Stack spacing={0.5}>
            <Typography variant="caption">{preview.name ? `Preview: ${preview.name}` : "Audio preview"}</Typography>
            <PreviewMeters levels={preview.levels} />
          </Stack>
        )}
        {salvos && (
          <Stack direction={{ xs: "column", md: "row" }} spacing={1} alignItems={{ md: "center" }}>
            <TextField select label="Select Salvo" value={preset?.id ?? ""}
              onChange={(event) => choosePreset(event.target.value)}
              disabled={controlsBusy || routeStateUnavailable || presets.isLoading || presets.isError}
              sx={{ minWidth: 220, flex: 1 }}>
              <MenuItem value="">Select Salvo</MenuItem>
              {presetOptions.map((saved) => (
                <MenuItem key={saved.id} value={saved.id}>{saved.name}</MenuItem>
              ))}
            </TextField>
            <Button variant="contained"
              disabled={controlsBusy || routeStateUnavailable || !preset || dirty ||
                stale || presetChanged || invalidSelection}
              onClick={() => preset && take(selections, preset)}>Take</Button>
            <Button variant="outlined"
              disabled={controlsBusy || routeStateUnavailable || selections.length === 0 || invalidSelection}
              onClick={() => {
                setSaveName(preset?.name ?? "");
                save.reset();
                setSaveOpen(true);
              }}>Save</Button>
            <Button disabled={controlsBusy || !preset || routeStateUnavailable || presets.isError}
              onClick={() => preset && choosePreset(preset.id)}>Reload salvo</Button>
            <Button color="error" disabled={controlsBusy || !preset}
              onClick={() => { remove.reset(); setDeleteOpen(true); }}>Delete</Button>
            <Typography variant="caption" sx={{ whiteSpace: "nowrap" }}>
              {selections.length} selected
            </Typography>
          </Stack>
        )}
        {presets.isError && salvos && (
          <Alert severity="error" action={<Button color="inherit"
            onClick={() => void presets.refetch()}>Retry</Button>}>
            Salvos could not be loaded: {presets.error.message}
          </Alert>
        )}
        {(sourceError || destinationError) && (
          <Alert severity="error">Endpoint catalog unavailable: {sourceError || destinationError}</Alert>
        )}
        {salvos && dirty && selections.length > 0 && (
          <Alert severity="info">Save the selected routes before taking this salvo.</Alert>
        )}
        {salvos && (stale || presetChanged) && (
          <Alert severity="warning">
            {presetChanged ? "The saved salvo changed or was deleted." : "A destination changed after this salvo was loaded."}
            {" "}Reload the salvo to review current state before taking it.
          </Alert>
        )}
        {salvos && invalidSelection && (
          <Alert severity="warning">This selection contains an unavailable or unvalidated input/output.</Alert>
        )}
        {operation.isError && (
          <Alert severity="error" action={unknownOutcome && operation.variables
            ? <Button color="inherit" disabled={operation.isPending}
                onClick={() => operation.mutate(operation.variables!)}>Retry safely</Button>
            : undefined}>
            {operation.error instanceof ApiRequestError && operation.error.status === 409
              ? "Route or salvo state changed. Reload and review your selection; no changes from this request were applied."
              : `The command could not be confirmed: ${operation.error.message}.`}
            {unknownOutcome && " Retry with the same operation key before issuing another command."}
          </Alert>
        )}
        {status.isError && operation.data && (
          <Alert severity="error" action={<Button color="inherit"
            onClick={() => void status.refetch()}>Check status</Button>}>
            Operation {operation.data.operationId} was accepted, but its status could not be refreshed.
          </Alert>
        )}
        {result && (
          <Alert severity={result.state === "completed" ? "success"
            : result.state === "failed" ? "error" : result.state === "partial" ? "warning" : "info"}>
            {result.state === "completed" ? "Command completed."
              : result.state === "partial" ? "Command partially completed. Review the destination outcomes."
              : result.state === "failed" ? "Command failed. Review the destination outcomes."
              : result.state === "desired_state_applied" ? "Desired state saved; worker execution is not confirmed."
              : "Command accepted; waiting for worker execution."}
            {" "}Operation {result.operationId}.
          </Alert>
        )}
        <CrosspointMatrix sources={sources} destinations={destinations}
          routes={routes} routesLoading={routesLoading} routesError={routesError}
          selectedSourceId="" selectedDestinationId="" selections={selections} selectionOnly={salvos}
          onSelect={select} onContextMenu={salvos ? undefined : openMenu} />
        {(result?.results?.length ?? 0) > 0 && (
          <Accordion disableGutters>
            <AccordionSummary>Destination outcomes ({result!.results!.length})</AccordionSummary>
            <AccordionDetails>
              <Stack spacing={1}>
                {result!.results!.map((item) => (
                  <DestinationOutcome key={item.destinationId} item={item}
                    name={destinations.find((destination) => destination.id === item.destinationId)?.friendlyName ?? item.destinationId} />
                ))}
              </Stack>
            </AccordionDetails>
          </Accordion>
        )}
      </Stack>
      <Menu open={menu !== null} onClose={() => setMenu(null)}
        anchorReference="anchorPosition" anchorPosition={menu ? { top: menu.top, left: menu.left } : undefined}>
        {onPreview && (
          <MenuItem disabled={controlsBusy || preview?.busy} onClick={() => {
            if (!menu) return;
            onPreview(menu.selection.sourceId);
            setMenu(null);
          }}>Preview input</MenuItem>
        )}
        <MenuItem disabled={controlsBusy || !menuCanTake}
          onClick={() => menu && take([menu.selection])}>Take</MenuItem>
        <MenuItem disabled={controlsBusy || !menuCanUnroute}
          onClick={() => {
            if (!menu) return;
            operation.mutate({ kind: "unroute", destinationId: menu.selection.destinationId,
              expectedRouteRevision: menu.selection.expectedRouteRevision, key: createPipelineId() });
            setMenu(null);
          }}>Unroute</MenuItem>
      </Menu>
      <Dialog open={confirmation !== null} onClose={() => setConfirmation(null)}>
        <DialogTitle>Replace existing routes?</DialogTitle>
        <DialogContent>
          <DialogContentText>
            This Take replaces an existing input on one or more outputs. ARC prepares
            each replacement before stopping the old route, then performs a hard cut.
            If Start fails, ARC attempts to restore the previous route. Other
            destinations execute independently; this is not an all-or-nothing media switch.
          </DialogContentText>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setConfirmation(null)}>Cancel</Button>
          <Button variant="contained" disabled={controlsBusy} onClick={() => {
            if (confirmation) operation.mutate(confirmation);
            setConfirmation(null);
          }}>Confirm Take</Button>
        </DialogActions>
      </Dialog>
      <Dialog open={saveOpen} onClose={() => !save.isPending && setSaveOpen(false)}>
        <DialogTitle>Save salvo</DialogTitle>
        <DialogContent>
          <TextField autoFocus margin="dense" label="Salvo name" fullWidth
            value={saveName} disabled={save.isPending}
            onChange={(event) => setSaveName(event.target.value)}
            slotProps={{ htmlInput: { maxLength: 100 } }} />
          <DialogContentText>{selections.length} selected action(s). Live routes will not change.</DialogContentText>
          {save.isError && <Alert severity="error">Unable to save salvo: {save.error.message}</Alert>}
        </DialogContent>
        <DialogActions>
          <Button disabled={save.isPending} onClick={() => setSaveOpen(false)}>Cancel</Button>
          <Button variant={preset ? "text" : "contained"}
            disabled={save.isPending || !saveName.trim()} onClick={() => saveDraft(false)}>
            Save as new
          </Button>
          {preset && <Button variant="contained" disabled={save.isPending || !saveName.trim()}
            onClick={() => saveDraft(true)}>Update salvo</Button>}
        </DialogActions>
      </Dialog>
      <Dialog open={deleteOpen} onClose={() => !remove.isPending && setDeleteOpen(false)}>
        <DialogTitle>Delete salvo?</DialogTitle>
        <DialogContent>
          <DialogContentText>Delete "{preset?.name}"? Live routes will not change.</DialogContentText>
          {remove.isError && <Alert severity="error">Unable to delete salvo: {remove.error.message}</Alert>}
        </DialogContent>
        <DialogActions>
          <Button disabled={remove.isPending} onClick={() => setDeleteOpen(false)}>Cancel</Button>
          <Button color="error" disabled={remove.isPending} onClick={() => preset && remove.mutate(preset)}>
            Delete salvo
          </Button>
        </DialogActions>
      </Dialog>
    </Paper>
  );
}
