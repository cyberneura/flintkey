<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { onMount } from 'svelte';
  import { iconLoader, pickIcon, type IconRef, type IconState } from './lib/icons';
  import TypeIcon from './lib/TypeIcon.svelte';
  import { intentFor, moveSelection } from './lib/keys';
  import { latestOnly } from './lib/latest';

  interface Row {
    id: string;
    title: string;
    keywords: string[];
    hotkey: string | null;
    kind: string;
    detail: string;
    /** Pictures to show, best first. When none loads, the row shows the icon of its `kind`. */
    icons: IconRef[];
  }

  interface Refreshed {
    rows: Row[];
    problems: string[];
  }

  interface View extends Refreshed {
    notices: string[];
    config_dir: string;
    showing: number;
  }

  let query = $state('');
  let rows = $state<Row[]>([]);
  let selected = $state(0);
  /** Failures from while the window was hidden, then the problems in the configuration. */
  let notices = $state<string[]>([]);
  let configProblems = $state<string[]>([]);
  const problems = $derived([...notices, ...configProblems]);
  let configDir = $state('');
  let input: HTMLInputElement | undefined = $state();
  let list: HTMLUListElement | undefined = $state();
  /** Image URL (or `failed`) by icon, filled in as icons arrive. */
  let iconStates = $state<Record<string, IconState>>({});
  const icons = iconLoader(
    (icon) => invoke<ArrayBuffer>('icon', { icon }),
    (key, state) => (iconStates[key] = state),
  );

  $effect(() => {
    icons.load(rows.flatMap((row) => row.icons));
  });

  // Number of the current showing (0 while hidden). Sent with run requests so the app can match it against
  // the current showing. Prevents actions from a previous showing from running after the window reopens.
  let showing = 0;

  // The list and the showing number come from the same response. Stale responses are dropped whole,
  // so the number never goes stale on its own.
  const results = latestOnly<{ rows: Row[]; showing?: number }>((found) => {
    rows = found.rows;
    selected = 0;
    if (found.showing !== undefined) {
      showing = found.showing;
    }
  });

  async function openSession() {
    // Focus before anything is fetched: a key pressed before the input has focus is lost, and the list may take
    // a moment (the files are read after the window appears).
    input?.focus();
    // This can be called for a notice while the window is open. Fetch the list for the text already typed.
    const view = invoke<View>('open_session', { query });
    results.start(view.then((opened) => ({ rows: opened.rows, showing: opened.showing })));
    const opened = await view;
    notices = opened.notices;
    configProblems = opened.problems;
    configDir = opened.config_dir;
  }

  /** The commands and apps were loaded again: search for the text typed so far in the new list. */
  async function refresh() {
    const refreshed = invoke<Refreshed>('refresh', { query });
    results.start(refreshed.then((found) => ({ rows: found.rows })));
    configProblems = (await refreshed).problems;
  }

  function search(text: string) {
    results.start(invoke<Row[]>('search', { query: text }).then((found) => ({ rows: found })));
  }

  async function run(row: Row | undefined) {
    if (!row) {
      return;
    }
    try {
      await invoke('run', { id: row.id, showing });
    } catch (error) {
      notices = [String(error)];
      configProblems = [];
    }
  }

  // If Enter is pressed before results for the typed text arrive, do not run a row from the old list.
  async function runSelected() {
    const pressedIn = showing;
    await results.settled();
    if (pressedIn === showing) {
      run(rows[selected]);
    }
  }

  function onKeydown(event: KeyboardEvent) {
    // Clicking an error message or similar takes focus from the input. Return it on the next key.
    // Not for modifier shortcuts such as copy (returning focus would clear the selection).
    if (document.activeElement !== input && !event.metaKey && !event.ctrlKey) {
      input?.focus();
    }
    const intent = intentFor(event);
    if (!intent) {
      return;
    }
    event.preventDefault();
    if (intent === 'run') {
      runSelected();
    } else if (intent === 'hide') {
      invoke('hide');
    } else if (intent === 'settings') {
      invoke('open_settings');
    } else {
      selected = moveSelection(selected, rows.length, intent === 'next' ? 1 : -1);
    }
  }

  function onBackdropMousedown(event: MouseEvent) {
    if (event.target === event.currentTarget) {
      invoke('hide');
    }
  }

  $effect(() => {
    list?.children[selected]?.scrollIntoView({ block: 'nearest' });
  });

  onMount(() => {
    const unlisten = [
      listen<number>('launcher-shown', (event) => {
        showing = event.payload;
        openSession();
      }),
      // The list read after this showing began, or after the files changed on disk, is ready. A hidden window
      // loads it when shown.
      listen('launcher-refreshed', () => {
        if (showing !== 0) refresh();
      }),
      listen('config-changed', () => {
        // An icon file may have been replaced or an icon value fixed.
        icons.reset();
        Object.values(iconStates).forEach((state) => state !== 'failed' && URL.revokeObjectURL(state));
        iconStates = {};
        if (showing !== 0) refresh();
      }),
      // Clear on hide so the previous input does not flash when the window next appears.
      listen('launcher-hidden', () => {
        showing = 0;
        query = '';
        selected = 0;
        search('');
      }),
    ];
    openSession();
    return () => unlisten.forEach((pending) => pending.then((stop) => stop()));
  });
</script>

<svelte:window onkeydown={onKeydown} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="flex h-screen w-screen flex-col p-5 select-none" onmousedown={onBackdropMousedown}>
  <div
    class="flex max-h-full flex-col overflow-hidden rounded-2xl border border-zinc-600/60 bg-zinc-900/95 text-zinc-100 shadow-xl"
  >
    <input
      bind:this={input}
      bind:value={query}
      oninput={(event) => search(event.currentTarget.value)}
      data-annotate="input-query"
      class="w-full shrink-0 bg-transparent px-5 py-4 text-xl outline-none placeholder:text-zinc-500"
      placeholder="Search commands and apps"
      autocomplete="off"
      autocorrect="off"
      autocapitalize="off"
      spellcheck="false"
    />

    {#if rows.length > 0}
      <ul bind:this={list} role="listbox" class="min-h-0 overflow-y-auto border-t border-zinc-700 py-1">
        {#each rows as row, index (row.id)}
          {@const picture = pickIcon(row.icons, iconStates)}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <li
            data-annotate="row-{row.id}"
            role="option"
            aria-selected={index === selected}
            class="mx-1 flex cursor-default items-center gap-3 rounded-lg px-4 py-2 {index === selected
              ? 'bg-amber-500/25'
              : ''}"
            onmousemove={() => (selected = index)}
            onclick={() => run(row)}
          >
            <div class="flex size-8 shrink-0 items-center justify-center">
              {#if picture}
                <img src={picture} alt="" class="size-8 object-contain" draggable="false" />
              {:else}
                <TypeIcon kind={row.kind} />
              {/if}
            </div>
            <div class="flex min-w-0 flex-1 flex-col">
              <div class="flex items-center gap-2">
                <span class="truncate">{row.title}</span>
                {#each row.keywords as keyword (keyword)}
                  <span
                    class="shrink-0 rounded bg-zinc-700 px-1.5 text-xs text-zinc-300"
                  >
                    {keyword}
                  </span>
                {/each}
              </div>
              <span class="truncate text-xs text-zinc-400">{row.detail}</span>
            </div>
            <div class="flex shrink-0 flex-col items-end gap-0.5">
              <span class="text-xs text-zinc-500">{row.kind}</span>
              {#if row.hotkey}
                <span class="font-mono text-xs text-zinc-500">{row.hotkey}</span>
              {/if}
            </div>
          </li>
        {/each}
      </ul>
    {:else}
      <div class="border-t border-zinc-700 px-5 py-4 text-sm text-zinc-400">
        {#if query.trim() === ''}
          <p>No commands yet. Add one YAML file per command to:</p>
          <p class="select-text">{configDir}/commands</p>
        {:else}
          <p>Nothing matches.</p>
        {/if}
      </div>
    {/if}

    {#if problems.length > 0}
      <ul
        data-annotate="problems"
        class="max-h-24 shrink-0 overflow-y-auto border-t border-red-900 bg-red-950/60 px-5 py-2 text-xs text-red-300 select-text"
      >
        {#each problems as problem}
          <li>{problem}</li>
        {/each}
      </ul>
    {/if}
  </div>
</div>
