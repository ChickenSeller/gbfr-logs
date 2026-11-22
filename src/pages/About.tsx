import { Anchor, Box, Code, Divider, ScrollArea, Stack, Text, Title } from "@mantine/core";
import { getVersion } from "@tauri-apps/api/app";
import { open } from "@tauri-apps/api/shell";
import { useEffect, useState } from "react";

// Bundled at build time, so the window shows exactly the licence shipped with the source.
import licenseText from "../../LICENSE?raw";

const REPOSITORY_URL = "https://github.com/ChickenSeller/gbfr-logs";
const AWA_EDITION_URL = "https://github.com/onelittlechildawa/gbfr-logs";
const ORIGINAL_URL = "https://github.com/false-spring/gbfr-logs";

/** Opens links in the default browser instead of inside the WebView. */
const ExternalLink = ({ href, children }: { href: string; children: string }) => (
  <Anchor
    href={href}
    onClick={(event) => {
      event.preventDefault();
      open(href);
    }}
  >
    {children}
  </Anchor>
);

/** About window, opened from the tray menu: version, credits and the MIT licence. */
export const AboutPage = () => {
  const [version, setVersion] = useState("");

  useEffect(() => {
    getVersion()
      .then(setVersion)
      .catch(() => {});
  }, []);

  return (
    <Box p="md" h="100vh">
      <Stack gap="xs" h="100%">
        <Title order={3}>
          GBFR Logs Kaguya Edition{" "}
          {version && (
            <Text span c="dimmed">
              {version}
            </Text>
          )}
        </Title>
        <Text size="sm">
          Maintained and modified by <ExternalLink href={REPOSITORY_URL}>ChickenSeller</ExternalLink>.
        </Text>
        <Text size="sm">
          Based on <ExternalLink href={AWA_EDITION_URL}>GBFR Logs Awa Edition</ExternalLink> by onelittlechildawa and
          the original open-source <ExternalLink href={ORIGINAL_URL}>GBFR Logs</ExternalLink> by False Spring.
        </Text>
        <Divider my="xs" label="License" labelPosition="left" />
        <ScrollArea flex={1} type="auto">
          <Code block>{licenseText}</Code>
        </ScrollArea>
      </Stack>
    </Box>
  );
};
