import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { ResourceRow } from "./App";
import type { SourceRecord } from "./api";

const source: SourceRecord = {
  id: "input",
  friendlyName: "Input",
  sourceType: "alsa_l16",
  desiredState: "disabled",
  currentRevision: 1,
  revisionId: "revision",
  lifecycle: "validated",
  configuration: { kind: "alsa_l16", device: "hw:1,0" },
};

describe("resource status", () => {
  it("shows a validated configuration as Active regardless of assignment state", () => {
    const html = renderToStaticMarkup(createElement(ResourceRow, { record: source }));
    expect(html).toContain("Active");
    expect(html).not.toContain("Standby");
    expect(html).not.toContain(">disabled<");
    expect(html).not.toContain("validated");
    const enabled = renderToStaticMarkup(createElement(ResourceRow, {
      record: { ...source, desiredState: "enabled" },
    }));
    expect(enabled).toContain(">Active<");
    expect(enabled).not.toContain("Enabled by assignment");
  });

  it("shows a draft needs validation without an assignment label", () => {
    const html = renderToStaticMarkup(createElement(ResourceRow, {
      record: { ...source, lifecycle: "draft", desiredState: "enabled" },
    }));
    expect(html).toContain("Draft - validation needed");
    expect(html).not.toContain("Enabled by assignment");
    expect(html).not.toContain("Active");
  });

  it("does not describe a retired configuration as Active", () => {
    const html = renderToStaticMarkup(createElement(ResourceRow, {
      record: { ...source, lifecycle: "retired" },
    }));
    expect(html).toContain("Configuration retired");
    expect(html).not.toContain(">Active<");
  });
});
