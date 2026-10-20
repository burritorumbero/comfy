import { ActivityIndicator, StyleSheet, Text, View } from "react-native";

import type { Services } from "../lib/api";
import {
  imagesOffText,
  llmLoadedText,
  statusOf,
  switchingText,
} from "../lib/services";
import { useRequestSwitch } from "../lib/servicesStore";
import { colors, maxFontScale, radius } from "../lib/theme";
import { useNow } from "../lib/useNow";
import { Banner } from "./Banner";

const paragraphs = (...parts: (string | null)[]) =>
  parts.filter(Boolean).join("\n\n");

/**
 * Shown on the feed whenever the Mac isn't ready for images, or the server did something by
 * itself: what holds the memory, and the way back.
 */
export function ServicesCard({ services }: { services: Services }) {
  const requestSwitch = useRequestSwitch();
  const status = statusOf(services);
  const now = useNow(status === "switching");

  if (status === "switching") {
    const text = switchingText(services, now);
    return (
      <View
        style={styles.card}
        accessibilityRole="progressbar"
        accessibilityLabel={text ?? undefined}
      >
        <ActivityIndicator color={colors.accent} />
        <Text style={styles.text} maxFontSizeMultiplier={maxFontScale}>
          {text}
        </Text>
      </View>
    );
  }
  const failed = services.error ? `Couldn't switch: ${services.error}` : null;
  if (status === "llm") {
    return (
      <Banner
        tone={failed ? "danger" : "warn"}
        text={paragraphs(failed, services.notice, llmLoadedText(services))}
      />
    );
  }
  if (status === "off") {
    const mode = services.mode;
    return (
      <Banner
        tone={failed ? "danger" : "warn"}
        text={paragraphs(failed, services.notice, imagesOffText(services))}
        action="Start ComfyUI"
        onAction={() => void requestSwitch(mode)}
      />
    );
  }
  if (failed) return <Banner tone="danger" text={failed} />;
  return services.notice ? <Banner tone="warn" text={services.notice} /> : null;
}

const styles = StyleSheet.create({
  card: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    marginHorizontal: 12,
    marginBottom: 8,
    padding: 12,
    borderRadius: radius.control,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: colors.accent,
    backgroundColor: colors.surface,
  },
  text: { color: colors.text, fontSize: 14, lineHeight: 19, flexShrink: 1 },
});
