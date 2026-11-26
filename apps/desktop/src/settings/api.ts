import { invoke } from '@tauri-apps/api/core';
import type { Os } from '../lib/hotkey';

export interface SettingsForm {
  hotkey: string;
  display: string;
  terminal: string | null;
}

export interface SettingsPage {
  form: SettingsForm;
  source: string;
  problems: string[];
  config_dir: string;
  os: Os;
  readable: boolean;
}

export interface HotkeyCheck {
  error: string | null;
  warnings: string[];
}

export interface CommandForm {
  title: string;
  keywords: string[];
  hotkey: string;
  kind: string;
  url: string;
  app: string;
  args: string[];
  run: string;
  cwd: string;
  env: [string, string][];
  pin: boolean;
  tags: string[];
  os: string[];
}

export interface CommandSummary {
  id: string;
  title: string;
  kind: string;
  keywords: string[];
  hotkey: string | null;
  tags: string[];
  os: string[];
  broken: boolean;
}

export interface CommandPage {
  id: string;
  form: CommandForm | null;
  source: string;
  problems: string[];
}

export const emptyCommand = (): CommandForm => ({
  title: '',
  keywords: [],
  hotkey: '',
  kind: 'url',
  url: '',
  app: '',
  args: [],
  run: '',
  cwd: '',
  env: [],
  pin: false,
  tags: [],
  os: [],
});

/** Errors from these commands are a list of problems. Turn whatever arrives into that shape. */
export function problemsOf(error: unknown): string[] {
  if (Array.isArray(error)) {
    return error.map(String);
  }
  return [String(error)];
}

export const api = {
  settingsPage: () => invoke<SettingsPage>('settings_page'),
  // `base` is the file text the page loaded; the app refuses the save if the file has changed since.
  settingsSave: (form: SettingsForm, base: string) => invoke<SettingsPage>('settings_save', { form, base }),
  settingsSaveSource: (source: string, base: string) =>
    invoke<SettingsPage>('settings_save_source', { source, base }),
  hotkeyCheck: (hotkey: string, launcher: boolean, id: string | null) =>
    invoke<HotkeyCheck>('hotkey_check', { hotkey, launcher, id }),
  commandsList: () => invoke<CommandSummary[]>('commands_list'),
  commandPage: (id: string) => invoke<CommandPage>('command_page', { id }),
  commandPreview: (id: string, form: CommandForm) => invoke<string>('command_preview', { id, form }),
  commandParse: (source: string) => invoke<CommandForm>('command_parse', { source }),
  // `base` is the file text the page loaded, or `null` for a new command (whose id must be free).
  commandSave: (id: string, form: CommandForm, base: string | null) =>
    invoke<CommandPage>('command_save', { id, form, base }),
  commandSaveSource: (id: string, source: string, base: string | null) =>
    invoke<CommandPage>('command_save_source', { id, source, base }),
  commandDelete: (id: string) => invoke<void>('command_delete', { id }),
  commandTry: (id: string) => invoke<void>('command_try', { id }),
  reveal: (id: string | null) => invoke<void>('reveal', { id }),
  openKeyboardSettings: () => invoke<void>('open_keyboard_settings'),
};
