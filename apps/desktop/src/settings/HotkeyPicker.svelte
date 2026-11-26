<script lang="ts">
  import { onDestroy } from 'svelte';
  import {
    describeHotkey,
    EMPTY,
    formatHotkey,
    hotkeyFromKeyPress,
    KEY_OPTIONS,
    modifierLabels,
    parseHotkey,
    type Hotkey,
    type Os,
  } from '../lib/hotkey';
  import { api, type HotkeyCheck } from './api';

  interface Props {
    value: string;
    os: Os;
    /** True for the launcher's hotkey. */
    launcher?: boolean;
    /** Id of the command being edited (`null` for a command with no file yet). */
    id?: string | null;
    /** Changes when the page reloads, so the check runs again (e.g. after a macOS shortcut was turned off). */
    epoch?: number;
    /** Commands may have no hotkey; the launcher must have one. */
    optional?: boolean;
    onchange: (value: string) => void;
    name: string;
  }

  let { value, os, launcher = false, id = null, epoch = 0, optional = false, onchange, name }: Props = $props();

  const labels = $derived(modifierLabels(os));
  // Modifiers chosen before a key. The saved text needs a key, so they are kept here until one is chosen.
  let pending = $state<Hotkey>({ ...EMPTY });
  const hotkey = $derived(value === '' ? pending : parseHotkey(value, os));
  const known = $derived(KEY_OPTIONS.some((option) => option.value === hotkey.key));
  let recording = $state(false);
  let check = $state<HotkeyCheck>({ error: null, warnings: [] });
  let root: HTMLDivElement | undefined = $state();

  // Modifiers picked for one command must not carry over to the next one opened.
  $effect(() => {
    void id;
    void launcher;
    pending = { ...EMPTY };
  });
  let checked = '';

  // Check the hotkey whenever it, the command it belongs to, or the page changes. Only the latest answer is shown.
  $effect(() => {
    const key = `${launcher}|${id}|${epoch}|${value}`;
    if (key === checked) {
      return;
    }
    checked = key;
    const current = value;
    api
      .hotkeyCheck(current, launcher, id)
      .then((result) => {
        if (checked === key) {
          check = result;
        }
      })
      .catch(() => {});
  });

  function update(change: Partial<Hotkey>) {
    const next = { ...hotkey, ...change };
    pending = next.key === '' ? next : { ...EMPTY };
    onchange(formatHotkey(next));
  }

  function toggle(modifier: 'ctrl' | 'alt' | 'shift' | 'cmd') {
    update({ [modifier]: !hotkey[modifier] });
  }

  function onRecordKey(event: KeyboardEvent) {
    // The pane may have been switched away while recording; never take keys typed elsewhere.
    if (!root || root.offsetParent === null) {
      stopRecording();
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    const noModifier = !event.ctrlKey && !event.altKey && !event.shiftKey && !event.metaKey;
    if (event.code === 'Escape' && noModifier) {
      stopRecording();
      return;
    }
    const pressed = hotkeyFromKeyPress(event);
    if (pressed === null) {
      return;
    }
    stopRecording();
    pending = { ...EMPTY };
    onchange(formatHotkey(pressed));
  }

  function startRecording() {
    recording = true;
    window.addEventListener('keydown', onRecordKey, true);
  }

  function stopRecording() {
    recording = false;
    window.removeEventListener('keydown', onRecordKey, true);
  }

  onDestroy(stopRecording);

  const modifiers = ['ctrl', 'alt', 'shift', 'cmd'] as const;
</script>

<div class="flex flex-col gap-2" bind:this={root}>
  <div class="flex flex-wrap items-center gap-2">
    {#each modifiers as modifier (modifier)}
      <button
        type="button"
        data-annotate="hotkey-{name}-{modifier}"
        aria-pressed={hotkey[modifier]}
        class="rounded-md border px-2.5 py-1 text-sm {hotkey[modifier]
          ? 'border-amber-500 bg-amber-500/25 text-amber-100'
          : 'border-zinc-600 bg-zinc-800 text-zinc-300 hover:bg-zinc-700'}"
        onclick={() => toggle(modifier)}
      >
        {labels[modifier]}
      </button>
    {/each}
    <span class="text-zinc-500">+</span>
    <select
      data-annotate="hotkey-{name}-key"
      class="rounded-md border border-zinc-600 bg-zinc-800 px-2 py-1 text-sm text-zinc-100"
      value={hotkey.key}
      onchange={(event) => update({ key: event.currentTarget.value })}
    >
      <option value="">Key…</option>
      {#if hotkey.key !== '' && !known}
        <option value={hotkey.key}>{hotkey.key}</option>
      {/if}
      {#each KEY_OPTIONS as option (option.value)}
        <option value={option.value}>{option.label}</option>
      {/each}
    </select>
    <button
      type="button"
      data-annotate="hotkey-{name}-record"
      class="rounded-md border px-2.5 py-1 text-sm {recording
        ? 'border-red-400 bg-red-500/20 text-red-200'
        : 'border-zinc-600 bg-zinc-800 text-zinc-300 hover:bg-zinc-700'}"
      onclick={() => (recording ? stopRecording() : startRecording())}
    >
      {recording ? 'Press keys… (Esc to cancel)' : 'Record'}
    </button>
    {#if optional && value !== ''}
      <button
        type="button"
        data-annotate="hotkey-{name}-clear"
        class="rounded-md px-2 py-1 text-sm text-zinc-400 hover:text-zinc-100"
        onclick={() => {
          pending = { ...EMPTY };
          onchange('');
        }}
      >
        Clear
      </button>
    {/if}
    {#if value !== ''}
      <span class="font-mono text-sm text-zinc-300">{describeHotkey(hotkey, os)}</span>
    {/if}
  </div>
  {#if recording}
    <p class="text-xs text-zinc-400">
      A hotkey that is already registered (by Flintkey or another app) never reaches this window. Pick it with the
      buttons instead.
    </p>
  {/if}
  {#if value === '' && (hotkey.ctrl || hotkey.alt || hotkey.shift || hotkey.cmd)}
    <p class="text-sm text-zinc-400">Now choose a key.</p>
  {:else if !optional && value === ''}
    <p class="text-sm text-red-300">Choose a key.</p>
  {/if}
  {#if check.error}
    <p class="text-sm text-red-300">{check.error}</p>
  {/if}
  {#each check.warnings as warning (warning)}
    <p class="text-sm text-amber-300">
      {warning}
      {#if os === 'macos' && warning.startsWith('macOS uses')}
        <button
          type="button"
          data-annotate="hotkey-{name}-keyboard-settings"
          class="ml-1 underline hover:text-amber-100"
          onclick={() => api.openKeyboardSettings()}
        >
          Open Keyboard Settings
        </button>
      {/if}
    </p>
  {/each}
</div>
