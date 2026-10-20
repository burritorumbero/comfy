import { DarkTheme, Stack, ThemeProvider } from 'expo-router';
import { StatusBar } from 'expo-status-bar';
import * as SystemUI from 'expo-system-ui';

import { PrivacyCover } from '../components/PrivacyCover';
import { ConnectionProvider } from '../lib/connection';
import { JobsProvider } from '../lib/jobsStore';
import { PreferencesProvider } from '../lib/preferences';
import { ServicesProvider } from '../lib/servicesStore';
import { colors } from '../lib/theme';

void SystemUI.setBackgroundColorAsync(colors.background);

const theme = {
  ...DarkTheme,
  colors: { ...DarkTheme.colors, background: colors.background, card: colors.surface, text: colors.text, border: colors.border, primary: colors.accent },
};

export default function RootLayout() {
  return (
    <ThemeProvider value={theme}>
      <ConnectionProvider>
        <PreferencesProvider>
          <JobsProvider>
            <ServicesProvider>
              <Stack
                screenOptions={{
                  headerStyle: { backgroundColor: colors.surface },
                  headerTintColor: colors.text,
                  headerShadowVisible: false,
                  contentStyle: { backgroundColor: colors.background },
                }}
              >
                <Stack.Screen name="index" options={{ headerShown: false }} />
                <Stack.Screen name="connect" options={{ title: 'Connection' }} />
                <Stack.Screen name="settings" options={{ title: 'Settings' }} />
                <Stack.Screen name="viewer" options={{ presentation: 'fullScreenModal', headerShown: false, animation: 'fade' }} />
              </Stack>
              <PrivacyCover />
            </ServicesProvider>
          </JobsProvider>
        </PreferencesProvider>
      </ConnectionProvider>
      <StatusBar style="light" />
    </ThemeProvider>
  );
}
