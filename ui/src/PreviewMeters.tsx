import { Box, Stack, Typography } from "@mui/material";

import type { StereoLevels } from "./pcmPreview";

export function PreviewMeters({ levels }: { levels: StereoLevels }) {
  return (
    <Stack direction="row" spacing={2} aria-label="Preview audio levels">
      {(["left", "right"] as const).map((channel) => (
        <Box key={channel} sx={{ flex: 1, minWidth: 0 }}>
          <Typography variant="caption">{channel === "left" ? "Left" : "Right"} {levels[channel].toFixed(1)} dBFS</Typography>
          <Box component="meter" min={-60} max={0} value={levels[channel]}
            aria-label={`${channel === "left" ? "Left" : "Right"} audio level in dBFS`}
            sx={{ display: "block", width: "100%" }} />
        </Box>
      ))}
    </Stack>
  );
}
