import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { SectionHeading, SidebarBrand } from "./App";

describe("workspace headings", () => {
  it("shows a centered, bold controller name without the ARC text heading", () => {
    const html = renderToStaticMarkup(createElement(SidebarBrand, { themeMode: "dark" }));
    expect(html).toContain("Audio Routing Controller");
    expect(html).toContain("text-align:center");
    expect(html).toContain("font-weight:700");
    expect(html).not.toContain(">ARC<");
    expect(html).toContain("/ARC-Light-128.png");
  });

  it("uses the shortened crosspoint subtitle and no admin subtitle", () => {
    const workspace = renderToStaticMarkup(createElement(SectionHeading, { section: "workspace" }));
    expect(workspace).toContain("Route connections and signal flow.");
    expect(workspace).not.toContain("for this workspace");

    const admin = renderToStaticMarkup(createElement(SectionHeading, { section: "admin" }));
    expect(admin).toContain("Admin Settings");
    expect(admin).not.toContain("<p");
  });
});
