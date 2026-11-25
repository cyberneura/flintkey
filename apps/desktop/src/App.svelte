<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { onMount } from 'svelte';
  import { intentFor, moveSelection } from './lib/keys';
  import { latestOnly } from './lib/latest';

  interface Row {
    id: string;
    title: string;
    keyword: string | null;
    kind: string;
    detail: string;
  }

  interface View {
    rows: Row[];
    problems: string[];
    config_dir: string;
    showing: number;
  }

  let query = $state('');
  let rows = $state<Row[]>([]);
  let selected = $state(0);
  let problems = $state<string[]>([]);
  let configDir = $state('');
  let input: HTMLInputElement | undefined = $state();
  let list: HTMLUListElement | undefined = $state();

  // 今の表示の番号 (隠れている間は 0)。実行要求に付けて送り、アプリ側が今の表示と照合する。
  // 窓を閉じて開き直したあとに、前の表示での操作が実行されるのを防ぐ。
  let showing = 0;

  // 一覧と表示の番号は同じ応答から取る。古い応答は丸ごと捨てるので、番号だけが古くなることは無い。
  const results = latestOnly<{ rows: Row[]; showing?: number }>((found) => {
    rows = found.rows;
    selected = 0;
    if (found.showing !== undefined) {
      showing = found.showing;
    }
  });

  async function openSession() {
    // 窓が出たまま知らせで呼ばれることがある。打ってある文字での一覧を受け取る。
    const view = invoke<View>('open_session', { query });
    results.start(view.then((opened) => ({ rows: opened.rows, showing: opened.showing })));
    const opened = await view;
    problems = opened.problems;
    configDir = opened.config_dir;
    input?.focus();
  }

  function search() {
    results.start(invoke<Row[]>('search', { query }).then((found) => ({ rows: found })));
  }

  async function run(row: Row | undefined) {
    if (!row) {
      return;
    }
    try {
      await invoke('run', { id: row.id, showing });
    } catch (error) {
      problems = [String(error)];
    }
  }

  // 打った文字の検索結果が届く前に Enter を押しても、古い一覧の行を実行しない。
  async function runSelected() {
    const pressedIn = showing;
    await results.settled();
    if (pressedIn === showing) {
      run(rows[selected]);
    }
  }

  function onKeydown(event: KeyboardEvent) {
    // エラー文などをクリックすると入力欄からフォーカスが外れる。次のキーで戻す。
    // コピーなどの修飾キー付きの操作では戻さない (戻すと選択範囲が消える)。
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
      // 次に出したとき、前回の入力が一瞬見えないように、隠した時点で消しておく。
      listen('launcher-hidden', () => {
        showing = 0;
        query = '';
        selected = 0;
        search();
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
      oninput={search}
      data-annotate="input-query"
      class="w-full shrink-0 bg-transparent px-5 py-4 text-xl outline-none placeholder:text-zinc-500"
      placeholder="Search commands"
      autocomplete="off"
      autocorrect="off"
      autocapitalize="off"
      spellcheck="false"
    />

    {#if rows.length > 0}
      <ul bind:this={list} role="listbox" class="min-h-0 overflow-y-auto border-t border-zinc-700 py-1">
        {#each rows as row, index (row.id)}
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
            <div class="flex min-w-0 flex-1 flex-col">
              <div class="flex items-center gap-2">
                <span class="truncate">{row.title}</span>
                {#if row.keyword}
                  <span
                    class="shrink-0 rounded bg-zinc-700 px-1.5 text-xs text-zinc-300"
                  >
                    {row.keyword}
                  </span>
                {/if}
              </div>
              <span class="truncate text-xs text-zinc-400">{row.detail}</span>
            </div>
            <span class="shrink-0 text-xs text-zinc-500">{row.kind}</span>
          </li>
        {/each}
      </ul>
    {:else}
      <div class="border-t border-zinc-700 px-5 py-4 text-sm text-zinc-400">
        {#if query.trim() === ''}
          <p>No commands yet. Add one YAML file per command to:</p>
          <p class="select-text">{configDir}/commands</p>
        {:else}
          <p>No command matches.</p>
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
