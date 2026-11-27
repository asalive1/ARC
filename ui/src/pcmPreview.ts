const SAMPLE_RATE = 48_000;
const MAX_PACKET_BYTES = 4096;
const MAX_BUFFER_SECONDS = 0.25;
const MAX_PENDING_BUFFERS = 32;
const METER_FLOOR_DB = -60;
const CONNECTION_TIMEOUT_MS = 10_000;

export interface StereoLevels {
  left: number;
  right: number;
}

export interface PreviewAudioOutput {
  readonly currentTime: number;
  schedule(left: Float32Array, right: Float32Array, start: number, ended: () => void): void;
  levels(): StereoLevels;
  close(): Promise<void>;
}

export function rmsDecibels(samples: Float32Array): number {
  if (samples.length === 0) return METER_FLOOR_DB;
  let sum = 0;
  for (const sample of samples) sum += sample * sample;
  const rms = Math.sqrt(sum / samples.length);
  return Math.max(METER_FLOOR_DB, Math.min(0, 20 * Math.log10(rms)));
}

export class PcmPreviewPlayer {
  private nextTime = 0;
  private pending = 0;
  private closed = false;
  private closing: Promise<void> | undefined;
  droppedPackets = 0;

  constructor(private readonly output: PreviewAudioOutput) {}

  enqueue(packet: ArrayBuffer): boolean {
    if (this.closed) throw new Error("Preview player is closed");
    if (packet.byteLength === 0 || packet.byteLength > MAX_PACKET_BYTES || packet.byteLength % 4 !== 0) {
      throw new Error("Invalid stereo PCM preview packet");
    }
    const now = this.output.currentTime;
    const start = Math.max(now, this.nextTime);
    const frames = packet.byteLength / 4;
    const end = start + frames / SAMPLE_RATE;
    if (this.pending >= MAX_PENDING_BUFFERS || end - now > MAX_BUFFER_SECONDS) {
      this.droppedPackets += 1;
      return false;
    }
    const bytes = new DataView(packet);
    const left = new Float32Array(frames);
    const right = new Float32Array(frames);
    for (let frame = 0; frame < frames; frame += 1) {
      left[frame] = bytes.getInt16(frame * 4, true) / 32768;
      right[frame] = bytes.getInt16(frame * 4 + 2, true) / 32768;
    }
    this.pending += 1;
    try {
      this.output.schedule(left, right, start, () => { this.pending -= 1; });
    } catch (error) {
      this.pending -= 1;
      throw error;
    }
    this.nextTime = end;
    return true;
  }

  levels(): StereoLevels {
    if (this.closed) throw new Error("Preview player is closed");
    return this.output.levels();
  }

  async close(): Promise<void> {
    if (!this.closing) {
      this.closed = true;
      this.closing = this.output.close().catch((error: unknown) => {
        this.closing = undefined;
        throw error;
      });
    }
    await this.closing;
  }
}

export class PcmPreviewStream {
  private closing: Promise<void> | undefined;
  private active = true;

  constructor(
    private readonly socket: WebSocket,
    private readonly player: PcmPreviewPlayer,
    private readonly onFailure: (error: Error) => void,
  ) {
    if (socket.readyState !== WebSocket.OPEN) {
      throw new Error("Preview connection is not open");
    }
    socket.binaryType = "arraybuffer";
    socket.addEventListener("message", this.onMessage);
    socket.addEventListener("error", this.onError);
    socket.addEventListener("close", this.onClose);
  }

  private readonly onMessage = (event: MessageEvent): void => {
    if (!this.active) return;
    try {
      if (!(event.data instanceof ArrayBuffer)) {
        throw new Error("Preview stream sent a non-binary message");
      }
      this.player.enqueue(event.data);
    } catch (error) {
      this.fail(error);
    }
  };

  private readonly onError = (): void => {
    this.fail(new Error("Preview connection failed"));
  };

  private readonly onClose = (event: CloseEvent): void => {
    const reasons: Record<string, string> = {
      preview_source_inactive: "This input must have a running route before it can be previewed.",
      preview_codec_unsupported: "This input's audio encoding is not supported for preview.",
      preview_capacity_exhausted: "The preview limit has been reached. Stop another preview and retry.",
      preview_session_revoked: "Your session ended. Sign in again to preview audio.",
      worker_session_unavailable: "The worker for this input is not connected.",
      worker_session_ended: "The worker connection ended.",
      preview_connection_timeout: "The worker did not connect in time.",
    };
    this.fail(new Error(reasons[event.reason] ??
      `Preview connection closed (${event.code})${event.reason ? `: ${event.reason}` : ""}`));
  };

  private fail(error: unknown): void {
    if (!this.active) return;
    void this.close().then(
      () => this.onFailure(error instanceof Error ? error : new Error(String(error))),
      (cleanupError: unknown) => this.onFailure(
        new AggregateError([error, cleanupError], "Preview failed and cleanup failed"),
      ),
    );
  }

  close(): Promise<void> {
    if (!this.closing) {
      this.active = false;
      this.socket.removeEventListener("message", this.onMessage);
      this.socket.removeEventListener("error", this.onError);
      this.socket.removeEventListener("close", this.onClose);
      this.closing = this.closeResources().catch((error: unknown) => {
        this.closing = undefined;
        throw error;
      });
    }
    return this.closing;
  }

  private async closeResources(): Promise<void> {
    const errors: unknown[] = [];
    try {
      if (this.socket.readyState === WebSocket.OPEN || this.socket.readyState === WebSocket.CONNECTING) {
        this.socket.close(1000, "Preview stopped");
      }
    } catch (error) {
      errors.push(error);
    }
    try {
      await this.player.close();
    } catch (error) {
      errors.push(error);
    }
    if (errors.length) throw new AggregateError(errors, "Preview cleanup failed");
  }
}

class WebAudioOutput implements PreviewAudioOutput {
  private readonly input: GainNode;
  private readonly left: AnalyserNode;
  private readonly right: AnalyserNode;
  private readonly leftSamples = new Float32Array(1024);
  private readonly rightSamples = new Float32Array(1024);

  constructor(private readonly context: AudioContext) {
    this.input = context.createGain();
    const splitter = context.createChannelSplitter(2);
    const silent = context.createGain();
    silent.gain.value = 0;
    this.left = context.createAnalyser();
    this.right = context.createAnalyser();
    this.left.fftSize = 1024;
    this.right.fftSize = 1024;
    this.input.connect(context.destination);
    this.input.connect(splitter);
    splitter.connect(this.left, 0);
    splitter.connect(this.right, 1);
    this.left.connect(silent);
    this.right.connect(silent);
    silent.connect(context.destination);
  }

  get currentTime(): number { return this.context.currentTime; }

  schedule(left: Float32Array, right: Float32Array, start: number, ended: () => void): void {
    if (this.context.state !== "running") throw new Error("Browser audio output is not running");
    const buffer = this.context.createBuffer(2, left.length, SAMPLE_RATE);
    buffer.getChannelData(0).set(left);
    buffer.getChannelData(1).set(right);
    const source = this.context.createBufferSource();
    source.buffer = buffer;
    source.connect(this.input);
    source.onended = () => { source.disconnect(); ended(); };
    try {
      source.start(start);
    } catch (error) {
      source.disconnect();
      throw error;
    }
  }

  levels(): StereoLevels {
    if (this.context.state !== "running") throw new Error("Browser audio output is not running");
    this.left.getFloatTimeDomainData(this.leftSamples);
    this.right.getFloatTimeDomainData(this.rightSamples);
    return { left: rmsDecibels(this.leftSamples), right: rmsDecibels(this.rightSamples) };
  }

  async close(): Promise<void> {
    await this.context.close();
  }
}

export async function openPcmPreviewPlayer(signal?: AbortSignal): Promise<PcmPreviewPlayer> {
  if (signal?.aborted) throw new Error("Preview connection cancelled");
  const context = new AudioContext();
  try {
    const resuming = context.resume();
    if (signal) {
      let abort = () => {};
      const cancelled = new Promise<never>((_resolve, reject) => {
        abort = () => reject(new Error("Preview connection cancelled"));
        signal.addEventListener("abort", abort);
        if (signal.aborted) abort();
      });
      try {
        await Promise.race([resuming, cancelled]);
      } finally {
        signal.removeEventListener("abort", abort);
      }
    } else {
      await resuming;
    }
    if (context.state !== "running") throw new Error("Browser audio output did not start");
    return new PcmPreviewPlayer(new WebAudioOutput(context));
  } catch (error) {
    try {
      await context.close();
    } catch (closeError) {
      throw new AggregateError([error, closeError], "Browser audio could not start or close");
    }
    throw error;
  }
}

/** The signal cancels opening; close the returned stream to stop an active preview. */
export async function connectPcmPreview(
  url: string,
  signal: AbortSignal,
  onFailure: (error: Error) => void,
): Promise<{ player: PcmPreviewPlayer; stream: PcmPreviewStream }> {
  if (signal.aborted) throw new Error("Preview connection cancelled");
  const player = await openPcmPreviewPlayer(signal);
  let socket: WebSocket | undefined;
  try {
    if (signal.aborted) throw new Error("Preview connection cancelled");
    socket = new WebSocket(url);
    const connection = socket;
    const stream = await new Promise<PcmPreviewStream>((resolve, reject) => {
      const cleanup = () => {
        clearTimeout(timeout);
        connection.removeEventListener("open", opened);
        connection.removeEventListener("error", failed);
        connection.removeEventListener("close", closed);
        signal.removeEventListener("abort", aborted);
      };
      const opened = () => {
        cleanup();
        try {
          resolve(new PcmPreviewStream(connection, player, onFailure));
        } catch (error) {
          reject(error);
        }
      };
      const failed = () => { cleanup(); reject(new Error("Preview connection failed")); };
      const closed = () => { cleanup(); reject(new Error("Preview connection closed before opening")); };
      const aborted = () => { cleanup(); reject(new Error("Preview connection cancelled")); };
      const timeout = setTimeout(() => {
        cleanup();
        reject(new Error("Preview connection timed out"));
      }, CONNECTION_TIMEOUT_MS);
      connection.addEventListener("open", opened);
      connection.addEventListener("error", failed);
      connection.addEventListener("close", closed);
      signal.addEventListener("abort", aborted);
      if (signal.aborted) aborted();
    });
    if (signal.aborted) {
      await stream.close();
      throw new Error("Preview connection cancelled");
    }
    return { player, stream };
  } catch (error) {
    const errors: unknown[] = [error];
    try {
      if (socket?.readyState === WebSocket.CONNECTING || socket?.readyState === WebSocket.OPEN) {
        socket.close(1000, "Preview stopped");
      }
    } catch (closeError) {
      errors.push(closeError);
    }
    try {
      await player.close();
    } catch (closeError) {
      errors.push(closeError);
    }
    if (errors.length > 1) throw new AggregateError(errors, "Preview connection and cleanup failed");
    throw error;
  }
}
