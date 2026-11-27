import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  connectPcmPreview, openPcmPreviewPlayer, PcmPreviewPlayer, PcmPreviewStream, rmsDecibels,
  type PreviewAudioOutput, type StereoLevels,
} from "./pcmPreview";
import { PreviewMeters } from "./PreviewMeters";

class FakeOutput implements PreviewAudioOutput {
  currentTime = 0;
  scheduled: { left: Float32Array; right: Float32Array; start: number; ended: () => void }[] = [];
  closed = false;
  schedule(left: Float32Array, right: Float32Array, start: number, ended: () => void) {
    this.scheduled.push({ left, right, start, ended });
  }
  levels(): StereoLevels { return { left: -6, right: -60 }; }
  async close() { this.closed = true; }
}

function packet(frames: number): ArrayBuffer {
  const result = new ArrayBuffer(frames * 4);
  const data = new DataView(result);
  for (let i = 0; i < frames; i += 1) {
    data.setInt16(i * 4, 16384, true);
    data.setInt16(i * 4 + 2, -32768, true);
  }
  return result;
}

class FakeNode {
  connect(_destination: FakeNode, _output?: number) { return this; }
  disconnect() {}
}

class FakeGain extends FakeNode { gain = { value: 1 }; }
class FakeAnalyser extends FakeNode {
  fftSize = 0;
  getFloatTimeDomainData(samples: Float32Array) { samples.fill(0.5); }
}
class FakeSource extends FakeNode {
  buffer?: FakeBuffer;
  onended?: () => void;
  startTime?: number;
  start(time: number) { this.startTime = time; }
}
class FakeBuffer {
  channels: Float32Array[];
  constructor(length: number) {
    this.channels = [new Float32Array(length), new Float32Array(length)];
  }
  getChannelData(channel: number) { return this.channels[channel]; }
}
class FakeAudioContext {
  static current: FakeAudioContext;
  currentTime = 0;
  state = "suspended";
  destination = new FakeNode();
  sources: FakeSource[] = [];
  constructor() { FakeAudioContext.current = this; }
  createGain() { return new FakeGain(); }
  createChannelSplitter() { return new FakeNode(); }
  createAnalyser() { return new FakeAnalyser(); }
  createBuffer(_channels: number, frames: number, _rate: number) { return new FakeBuffer(frames); }
  createBufferSource() {
    const source = new FakeSource();
    this.sources.push(source);
    return source;
  }
  async resume() { this.state = "running"; }
  async close() { this.state = "closed"; }
}

class FakeWebSocket extends EventTarget {
  static OPEN = 1;
  static CONNECTING = 0;
  static current: FakeWebSocket;
  binaryType: BinaryType = "blob";
  readyState = FakeWebSocket.OPEN;
  closeCalls = 0;
  constructor() {
    super();
    FakeWebSocket.current = this;
  }
  close() {
    this.closeCalls += 1;
    this.readyState = 3;
  }
  message(data: unknown) {
    this.dispatchEvent(new MessageEvent("message", { data }));
  }
  disconnect(code: number, reason: string) {
    this.readyState = 3;
    const event = new Event("close");
    Object.defineProperties(event, { code: { value: code }, reason: { value: reason } });
    this.dispatchEvent(event);
  }
}

class FakeConnectingWebSocket extends FakeWebSocket {
  constructor() {
    super();
    this.readyState = FakeWebSocket.CONNECTING;
  }
  open() {
    this.readyState = FakeWebSocket.OPEN;
    this.dispatchEvent(new Event("open"));
  }
}

afterEach(() => vi.unstubAllGlobals());

describe("PCM preview playback", () => {
  it("uses browser audio output and meters the rendered stereo channels", async () => {
    vi.stubGlobal("AudioContext", FakeAudioContext);
    const player = await openPcmPreviewPlayer();
    expect(player.enqueue(packet(2))).toBe(true);
    const context = FakeAudioContext.current;
    expect(context.sources[0].startTime).toBe(0);
    expect(context.sources[0].buffer?.channels[0][0]).toBe(0.5);
    expect(context.sources[0].buffer?.channels[1][0]).toBe(-1);
    expect(player.levels().left).toBeCloseTo(-6.02, 2);
    context.sources[0].onended?.();
    await player.close();
    expect(context.state).toBe("closed");
  });

  it("reports browser audio suspension rather than silently buffering", async () => {
    vi.stubGlobal("AudioContext", FakeAudioContext);
    const player = await openPcmPreviewPlayer();
    FakeAudioContext.current.state = "suspended";
    expect(() => player.enqueue(packet(1))).toThrow("Browser audio output is not running");
    expect(() => player.levels()).toThrow("Browser audio output is not running");
    await player.close();
  });

  it("blocks new packets while closing and shares an in-flight close", async () => {
    const audio = new FakeOutput();
    let finishClose = () => {};
    const close = vi.spyOn(audio, "close").mockImplementation(() => new Promise<void>((resolve) => {
      finishClose = resolve;
    }));
    const player = new PcmPreviewPlayer(audio);
    const first = player.close();
    const second = player.close();
    expect(close).toHaveBeenCalledTimes(1);
    expect(() => player.enqueue(packet(1))).toThrow("Preview player is closed");
    finishClose();
    await Promise.all([first, second]);
  });

  it("allows a failed audio close to be retried", async () => {
    const audio = new FakeOutput();
    const close = vi.spyOn(audio, "close").mockRejectedValueOnce(new Error("close failed"));
    const player = new PcmPreviewPlayer(audio);
    await expect(player.close()).rejects.toThrow("close failed");
    await player.close();
    expect(close).toHaveBeenCalledTimes(2);
    expect(audio.closed).toBe(true);
  });

  it("decodes signed little-endian stereo and schedules continuous audio", () => {
    const output = new FakeOutput();
    const player = new PcmPreviewPlayer(output);
    expect(player.enqueue(packet(480))).toBe(true);
    expect(player.enqueue(packet(480))).toBe(true);
    expect(output.scheduled[0].left[0]).toBe(0.5);
    expect(output.scheduled[0].right[0]).toBe(-1);
    expect(output.scheduled[0].start).toBe(0);
    expect(output.scheduled[1].start).toBeCloseTo(0.01);
    output.currentTime = 1;
    expect(player.enqueue(packet(480))).toBe(true);
    expect(output.scheduled[2].start).toBe(1);
  });

  it("bounds queued audio and rejects malformed packets", async () => {
    const output = new FakeOutput();
    const player = new PcmPreviewPlayer(output);
    for (let i = 0; i < 11; i += 1) expect(player.enqueue(packet(1024))).toBe(true);
    expect(player.enqueue(packet(1024))).toBe(false);
    expect(player.droppedPackets).toBe(1);
    expect(() => player.enqueue(new ArrayBuffer(3))).toThrow("Invalid stereo PCM");
    expect(() => player.enqueue(new ArrayBuffer(4100))).toThrow("Invalid stereo PCM");
    output.scheduled[0].ended();
    output.currentTime = 0.1;
    expect(player.enqueue(packet(1024))).toBe(true);
    await player.close();
    expect(output.closed).toBe(true);
    expect(() => player.enqueue(packet(1))).toThrow("closed");
  });

  it("limits the number of short scheduled buffers", () => {
    const output = new FakeOutput();
    const player = new PcmPreviewPlayer(output);
    for (let i = 0; i < 32; i += 1) expect(player.enqueue(packet(1))).toBe(true);
    expect(player.enqueue(packet(1))).toBe(false);
    output.scheduled[0].ended();
    expect(player.enqueue(packet(1))).toBe(true);
  });
});

describe("PCM preview stream", () => {
  it("plays binary packets and closes the socket and player only once", async () => {
    vi.stubGlobal("WebSocket", FakeWebSocket);
    const socket = new WebSocket("ws://localhost");
    const fake = FakeWebSocket.current;
    const output = new FakeOutput();
    const failure = vi.fn();
    const stream = new PcmPreviewStream(socket, new PcmPreviewPlayer(output), failure);
    expect(fake.binaryType).toBe("arraybuffer");
    fake.message(packet(2));
    expect(output.scheduled[0].left[0]).toBe(0.5);
    expect(output.scheduled[0].right[0]).toBe(-1);
    await Promise.all([stream.close(), stream.close()]);
    fake.message(packet(2));
    expect(output.scheduled).toHaveLength(1);
    expect(fake.closeCalls).toBe(1);
    expect(output.closed).toBe(true);
    expect(failure).not.toHaveBeenCalled();
  });

  it("closes and reports malformed stream messages", async () => {
    vi.stubGlobal("WebSocket", FakeWebSocket);
    const socket = new WebSocket("ws://localhost");
    const output = new FakeOutput();
    const failure = vi.fn();
    new PcmPreviewStream(socket, new PcmPreviewPlayer(output), failure);
    FakeWebSocket.current.message(new ArrayBuffer(3));
    await vi.waitFor(() => expect(failure).toHaveBeenCalledTimes(1));
    expect(failure.mock.calls[0][0].message).toContain("Invalid stereo PCM");
    expect(FakeWebSocket.current.closeCalls).toBe(1);
    expect(output.closed).toBe(true);
  });

  it("bounds playback when stream packets arrive faster than audio plays", async () => {
    vi.stubGlobal("WebSocket", FakeWebSocket);
    const socket = new WebSocket("ws://localhost");
    const output = new FakeOutput();
    const player = new PcmPreviewPlayer(output);
    const failure = vi.fn();
    const stream = new PcmPreviewStream(socket, player, failure);
    for (let i = 0; i < 12; i += 1) FakeWebSocket.current.message(packet(1024));
    expect(output.scheduled).toHaveLength(11);
    expect(player.droppedPackets).toBe(1);
    expect(failure).not.toHaveBeenCalled();
    await stream.close();
  });

  it("reports remote disconnect and cleanup errors together", async () => {
    vi.stubGlobal("WebSocket", FakeWebSocket);
    const socket = new WebSocket("ws://localhost");
    const output = new FakeOutput();
    vi.spyOn(output, "close").mockRejectedValueOnce(new Error("audio close failed"));
    const failure = vi.fn();
    const stream = new PcmPreviewStream(socket, new PcmPreviewPlayer(output), failure);
    FakeWebSocket.current.disconnect(1008, "credential revoked");
    await vi.waitFor(() => expect(failure).toHaveBeenCalledTimes(1));
    const error = failure.mock.calls[0][0];
    expect(error).toBeInstanceOf(AggregateError);
    expect(error.errors[0].message).toContain("credential revoked");
    expect(error.errors[1].errors[0].message).toBe("audio close failed");
    await stream.close();
    expect(output.closed).toBe(true);
  });
});

describe("opening a PCM preview", () => {
  it("plays audio after the socket opens and closes both resources", async () => {
    vi.stubGlobal("AudioContext", FakeAudioContext);
    vi.stubGlobal("WebSocket", FakeConnectingWebSocket);
    const opening = connectPcmPreview("ws://localhost/preview", new AbortController().signal, vi.fn());
    await vi.waitFor(() => expect(FakeWebSocket.current?.readyState).toBe(FakeWebSocket.CONNECTING));
    const socket = FakeWebSocket.current as FakeConnectingWebSocket;
    socket.open();
    const { player, stream } = await opening;
    socket.message(packet(1));
    expect(FakeAudioContext.current.sources[0].buffer?.channels[0][0]).toBe(0.5);
    expect(player.levels().left).toBeCloseTo(-6.02, 2);
    await stream.close();
    expect(socket.closeCalls).toBe(1);
    expect(FakeAudioContext.current.state).toBe("closed");
  });

  it("closes pending audio and socket resources if opening is cancelled", async () => {
    vi.stubGlobal("AudioContext", FakeAudioContext);
    vi.stubGlobal("WebSocket", FakeConnectingWebSocket);
    const controller = new AbortController();
    const opening = connectPcmPreview("ws://localhost/preview", controller.signal, vi.fn());
    const rejected = expect(opening).rejects.toThrow("Preview connection cancelled");
    await vi.waitFor(() => expect(FakeWebSocket.current?.readyState).toBe(FakeWebSocket.CONNECTING));
    const socket = FakeWebSocket.current;
    controller.abort();
    await rejected;
    expect(socket.closeCalls).toBe(1);
    expect(FakeAudioContext.current.state).toBe("closed");
  });

  it("cancels a pending browser audio resume without opening a socket", async () => {
    class SuspendedAudioContext extends FakeAudioContext {
      override async resume() { await new Promise<void>(() => {}); }
    }
    vi.stubGlobal("AudioContext", SuspendedAudioContext);
    vi.stubGlobal("WebSocket", FakeConnectingWebSocket);
    const controller = new AbortController();
    const opening = connectPcmPreview("ws://localhost/preview", controller.signal, vi.fn());
    const rejected = expect(opening).rejects.toThrow("Preview connection cancelled");
    controller.abort();
    await rejected;
    expect(FakeAudioContext.current.state).toBe("closed");
  });

  it("closes pending resources if the socket fails to open", async () => {
    vi.stubGlobal("AudioContext", FakeAudioContext);
    vi.stubGlobal("WebSocket", FakeConnectingWebSocket);
    const opening = connectPcmPreview("ws://localhost/preview", new AbortController().signal, vi.fn());
    const rejected = expect(opening).rejects.toThrow("Preview connection failed");
    await vi.waitFor(() => expect(FakeWebSocket.current?.readyState).toBe(FakeWebSocket.CONNECTING));
    const socket = FakeWebSocket.current;
    socket.dispatchEvent(new Event("error"));
    await rejected;
    expect(socket.closeCalls).toBe(1);
    expect(FakeAudioContext.current.state).toBe("closed");
  });
});

describe("preview audio meters", () => {
  it("reports silence and a half-scale signal in dBFS", () => {
    expect(rmsDecibels(new Float32Array([0, 0]))).toBe(-60);
    expect(rmsDecibels(new Float32Array([0.5, -0.5]))).toBeCloseTo(-6.02, 2);
  });

  it("renders independent accessible stereo levels", () => {
    const html = renderToStaticMarkup(createElement(PreviewMeters, {
      levels: { left: -6, right: -60 },
    }));
    expect(html).toContain('aria-label="Left audio level in dBFS"');
    expect(html).toContain('aria-label="Right audio level in dBFS"');
    expect(html).toContain("-6.0 dBFS");
    expect(html).toContain("-60.0 dBFS");
  });
});
