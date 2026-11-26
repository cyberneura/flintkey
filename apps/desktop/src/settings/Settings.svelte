<script lang="ts">
  import { listen } from '@tauri-apps/api/event';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { onMount } from 'svelte';
  import type { Os } from '../lib/hotkey';
  import { api } from './api';
  import CommandsPane from './CommandsPane.svelte';
  import GeneralPane from './GeneralPane.svelte';

  let pane = $state<'general' | 'commands'>('general');
  let os = $state<Os>('macos');
  let reloadKey = $state(0);
  let commandsPane: CommandsPane | undefined = $state();

  onMount(() => {
    api.settingsPage().then((page) => (os = page.os)).catch(() => {});
    const shown = listen('settings-shown', () => reloadKey++);
    const failed = listen<string>('settings-try-failed', (event) => commandsPane?.showTryFailure(event.payload));
    return () => {
      shown.then((stop) => stop());
      failed.then((stop) => stop());
    };
  });

  // The app has no menu bar of its own, so close the window with ⌘W / Ctrl+W here.
  function onKeydown(event: KeyboardEvent) {
    const modifier = os === 'macos' ? event.metaKey : event.ctrlKey;
    if (modifier && event.key.toLowerCase() === 'w') {
      event.preventDefault();
      getCurrentWindow().close();
    }
  }

  const panes = [
    { value: 'general', label: 'General' },
    { value: 'commands', label: 'Commands' },
  ] as const;
</script>

<svelte:window onkeydown={onKeydown} />

<div class="flex h-screen bg-zinc-900 text-zinc-100">
  <nav class="flex w-44 shrink-0 flex-col gap-1 border-r border-zinc-800 bg-zinc-950/60 p-3">
    {#each panes as item (item.value)}
      <button
        type="button"
        data-annotate="settings-nav-{item.value}"
        class="rounded-md px-3 py-1.5 text-left text-sm {pane === item.value
          ? 'bg-zinc-700 text-zinc-50'
          : 'text-zinc-400 hover:bg-zinc-800 hover:text-zinc-100'}"
        onclick={() => (pane = item.value)}
      >
        {item.label}
      </button>
    {/each}
  </nav>
  <main class="min-w-0 flex-1 overflow-hidden p-6">
    <!-- Both stay mounted so switching panes keeps unsaved edits. -->
    <div class="h-full overflow-y-auto" class:hidden={pane !== 'general'}>
      <GeneralPane {reloadKey} />
    </div>
    <div class="h-full" class:hidden={pane !== 'commands'}>
      <CommandsPane bind:this={commandsPane} {os} {reloadKey} />
    </div>
  </main>
</div>
