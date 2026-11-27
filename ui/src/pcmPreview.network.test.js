import { createHash } from "node:crypto";
import { once } from "node:events";
import { createServer } from "node:http";
import { expect, it, vi } from "vitest";

import { PcmPreviewPlayer, PcmPreviewStream } from "./pcmPreview";

it("plays PCM received over a real local WebSocket", async () => {
  const server = createServer();
  const upgrade = new Promise((resolve, reject) => {
    server.once("upgrade", (request, socket) => {
      const key = request.headers["sec-websocket-key"];
      if (!key) {
        socket.destroy();
        reject(new Error("Missing WebSocket key"));
        return;
      }
      const accept = createHash("sha1")
        .update(`${key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`).digest("base64");
      socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
      resolve(socket);
    });
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  if (!address || typeof address === "string") throw new Error("Missing test server address");
  const socket = new WebSocket(`ws://127.0.0.1:${address.port}`);
  let peer;
  let stream;
  try {
    [peer] = await Promise.all([upgrade, once(socket, "open")]);
    const scheduled = [];
    const output = {
      currentTime: 0,
      schedule: (left, right) => scheduled.push({ left, right }),
      levels: () => ({ left: -60, right: -60 }),
      close: async () => {},
    };
    const failure = vi.fn();
    stream = new PcmPreviewStream(socket, new PcmPreviewPlayer(output), failure);
    const audio = Buffer.alloc(8);
    audio.writeInt16LE(16384, 0);
    audio.writeInt16LE(-32768, 2);
    peer.write(Buffer.concat([Buffer.from([0x82, audio.length]), audio]));
    await vi.waitFor(() => expect(scheduled).toHaveLength(1));
    expect(scheduled[0].left[0]).toBe(0.5);
    expect(scheduled[0].right[0]).toBe(-1);
    expect(failure).not.toHaveBeenCalled();
  } finally {
    if (stream) await stream.close();
    else socket.close();
    peer?.destroy();
    await new Promise((resolve, reject) => server.close((error) => error ? reject(error) : resolve()));
  }
});
