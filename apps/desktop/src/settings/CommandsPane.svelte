<script lang="ts">
  import { untrack } from 'svelte';
  import type { Os } from '../lib/hotkey';
  import { api, emptyCommand, problemsOf, type CommandForm, type CommandPage, type CommandSummary } from './api';
  import HotkeyPicker from './HotkeyPicker.svelte';
  import Problems from './Problems.svelte';
  import TokenInput from './TokenInput.svelte';

  let { os, reloadKey, changedKey }: { os: Os; reloadKey: number; changedKey: number } = $props();
  /** The files changed on disk while there were unsaved edits here. */
  let changedWhileEditing = $state(false);

  let commands = $state<CommandSummary[]>([]);
  let listProblems = $state<string[]>([]);
  let filter = $state('');
  let tag = $state('');

  /** The command being edited. `creating` means it has no file yet. */
  let selectedId = $state<string | null>(null);
  let creating = $state(false);
  let newId = $state('');
  let page = $state<CommandPage | null>(null);
  let form = $state<CommandForm>(emptyCommand());
  let source = $state('');
  /** The arguments box as typed. Kept apart from `form.args` so a line being typed is not cut off. */
  let argsText = $state('');
  let view = $state<'form' | 'yaml'>('form');
  let problems = $state<string[]>([]);
  let status = $state('');
  let busy = $state(false);
  let confirmDelete = $state(false);
  /** A command the user clicked while there were unsaved changes. */
  let pendingSwitch = $state<string | null | undefined>(undefined);

  const tags = $derived([...new Set(commands.flatMap((command) => command.tags))].sort());
  const shown = $derived(
    commands.filter((command) => {
      const text = filter.trim().toLowerCase();
      const matches =
        text === '' ||
        command.title.toLowerCase().includes(text) ||
        command.id.toLowerCase().includes(text) ||
        command.keywords.some((keyword) => keyword.toLowerCase().includes(text));
      return matches && (tag === '' || command.tags.includes(tag));
    }),
  );
  const savedForm = $derived(page?.form ? JSON.stringify(page.form) : null);
  const dirty = $derived(
    creating ||
      (page !== null &&
        (view === 'yaml' ? source !== page.source : savedForm === null || JSON.stringify(form) !== savedForm)),
  );

  async function loadList() {
    try {
      commands = await api.commandsList();
      listProblems = [];
    } catch (error) {
      listProblems = problemsOf(error);
    }
  }

  // Reload whenever the window is shown again or the files change on disk. Unsaved edits are kept.
  function reload() {
    loadList();
    if (selectedId !== null && !creating && !dirty) {
      open(selectedId, true);
    }
  }

  $effect(() => {
    void reloadKey;
    untrack(reload);
  });

  $effect(() => {
    void changedKey;
    untrack(() => {
      if (changedKey === 0) return;
      if (dirty && !creating) {
        changedWhileEditing = true;
      }
      reload();
    });
  });

  function showPage(loaded: CommandPage) {
    changedWhileEditing = false;
    page = loaded;
    form = loaded.form ? structuredClone($state.snapshot(loaded.form)) : emptyCommand();
    source = loaded.source;
    problems = loaded.problems;
    // A file that is not a valid command can only be fixed as YAML.
    view = loaded.form ? view : 'yaml';
  }

  /** `keepStatus`: a reload after this window's own save should not clear its "Saved." */
  async function open(id: string, keepStatus = false) {
    if (!keepStatus) status = '';
    confirmDelete = false;
    try {
      const loaded = await api.commandPage(id);
      selectedId = id;
      creating = false;
      showPage(loaded);
    } catch (error) {
      problems = problemsOf(error);
    }
  }

  function select(id: string | null) {
    if (dirty) {
      pendingSwitch = id;
      return;
    }
    switchTo(id);
  }

  function switchTo(id: string | null) {
    pendingSwitch = undefined;
    if (id === null) {
      startNew();
    } else {
      open(id);
    }
  }

  function startNew() {
    confirmDelete = false;
    selectedId = null;
    creating = true;
    newId = '';
    page = null;
    form = emptyCommand();
    argsText = '';
    source = '';
    view = 'form';
    problems = [];
    status = '';
  }

  /** A file name from the title, for a new command: lowercase, spaces to hyphens. */
  function suggestId(title: string): string {
    return title
      .trim()
      .toLowerCase()
      .replace(/[\s/\\:|]+/g, '-')
      .replace(/^[.-]+|-+$/g, '');
  }

  const idToSave = $derived(creating ? (newId.trim() || suggestId(form.title)) : (selectedId ?? ''));

  async function showYaml() {
    if (view === 'yaml') return;
    const asked = JSON.stringify(form);
    try {
      const preview = await api.commandPreview(idToSave || 'new', $state.snapshot(form));
      // The form may have been edited while the preview was on its way; that text would not include the edit.
      if (JSON.stringify(form) !== asked) return;
      source = preview;
      problems = [];
      view = 'yaml';
    } catch (error) {
      problems = problemsOf(error);
    }
  }

  async function showForm() {
    if (view === 'form') return;
    // The form cannot hold comments or formatting typed in the YAML, and switching back would rebuild the text
    // from the form. Ask for the YAML to be saved (or reverted) first, unless the command has no file yet.
    if (!creating && page !== null && source !== page.source) {
      problems = ['Save or revert the YAML before switching to the form; comments and layout typed here would be lost.'];
      return;
    }
    try {
      form = await api.commandParse(source);
      problems = [];
      view = 'form';
    } catch (error) {
      problems = problemsOf(error);
    }
  }

  async function save() {
    if (idToSave === '') {
      problems = ['Give the command an id (its file name).'];
      return;
    }
    busy = true;
    status = '';
    try {
      const base = creating ? null : (page?.source ?? null);
      const saved =
        view === 'yaml'
          ? await api.commandSaveSource(idToSave, source, base)
          : await api.commandSave(idToSave, $state.snapshot(form), base);
      selectedId = saved.id;
      creating = false;
      showPage(saved);
      status = saved.problems.length === 0 ? 'Saved.' : '';
      await loadList();
    } catch (error) {
      problems = problemsOf(error);
    } finally {
      busy = false;
    }
  }

  // Loads the file again, so a file changed outside the window can be picked up.
  function revert() {
    status = '';
    if (creating) {
      startNew();
    } else if (selectedId !== null) {
      open(selectedId);
    }
  }

  async function remove() {
    if (!selectedId) return;
    if (!confirmDelete) {
      confirmDelete = true;
      return;
    }
    try {
      await api.commandDelete(selectedId);
      selectedId = null;
      page = null;
      creating = false;
      confirmDelete = false;
      await loadList();
    } catch (error) {
      problems = problemsOf(error);
    }
  }

  async function tryIt() {
    if (!selectedId) return;
    status = '';
    try {
      await api.commandTry(selectedId);
      status = 'Started.';
    } catch (error) {
      problems = problemsOf(error);
    }
  }

  export function showTryFailure(message: string) {
    problems = [message];
    status = '';
  }

  const kinds = [
    { value: 'url', label: 'Open URL' },
    { value: 'app', label: 'Open app' },
    { value: 'script', label: 'Run script' },
    { value: 'terminal', label: 'Run in terminal' },
  ];
  const systems = [
    { value: 'macos', label: 'macOS' },
    { value: 'windows', label: 'Windows' },
    { value: 'linux', label: 'Linux' },
  ];

  // Only an edited box drops empty lines at the end (a new line is typed before its text). Untouched arguments,
  // including a meaningful trailing empty one, are saved as they were loaded.
  function argsFromText(text: string): string[] {
    const lines = text.split('\n');
    while (lines.length > 0 && lines[lines.length - 1] === '') {
      lines.pop();
    }
    return lines;
  }

  $effect(() => {
    // Refill the box whenever a different form is loaded (open, new, revert, YAML → form).
    void page;
    void view;
    void creating;
    untrack(() => (argsText = form.args.join('\n')));
  });

  function toggleOs(value: string) {
    form.os = form.os.includes(value) ? form.os.filter((os) => os !== value) : [...form.os, value];
  }

  const input =
    'w-full rounded-md border border-zinc-600 bg-zinc-800 px-2 py-1 text-sm text-zinc-100 outline-none focus:border-amber-500';
  const label = 'text-sm font-medium text-zinc-200';
</script>

<div class="flex h-full min-h-0 gap-4">
  <!-- List -->
  <div class="flex w-72 shrink-0 flex-col gap-2">
    <div class="flex gap-2">
      <input data-annotate="commands-filter" class={input} placeholder="Filter" bind:value={filter} />
      <button
        type="button"
        data-annotate="commands-new"
        class="shrink-0 rounded-md bg-zinc-700 px-2.5 text-sm text-zinc-100 hover:bg-zinc-600"
        onclick={() => select(null)}
      >
        New
      </button>
    </div>
    <select data-annotate="commands-tag" class={input} bind:value={tag}>
      <option value="">All tags ({commands.length})</option>
      {#each tags as name (name)}
        <option value={name}>{name}</option>
      {/each}
    </select>
    <Problems problems={listProblems} />
    <ul class="min-h-0 flex-1 overflow-y-auto rounded-md border border-zinc-800" role="listbox">
      {#each shown as command (command.id)}
        <li role="option" aria-selected={command.id === selectedId}>
          <button
            type="button"
            data-annotate="commands-item-{command.id}"
            class="flex w-full flex-col items-start gap-0.5 px-3 py-1.5 hover:bg-zinc-800 {command.id === selectedId
              ? 'bg-amber-500/20'
              : ''}"
            onclick={() => select(command.id)}
          >
            <span class="flex w-full items-center gap-2">
              <span class="truncate text-sm {command.broken ? 'text-red-300' : 'text-zinc-100'}">{command.title}</span>
              {#if command.broken}<span class="shrink-0 text-xs text-red-400">broken</span>{/if}
            </span>
            <span class="flex w-full items-center justify-between gap-2 text-xs text-zinc-500">
              <span class="truncate">{command.id}</span>
              {#if command.hotkey}<span class="shrink-0 font-mono">{command.hotkey}</span>{/if}
            </span>
          </button>
        </li>
      {/each}
    </ul>
  </div>

  <!-- Editor -->
  <div class="flex min-h-0 min-w-0 flex-1 flex-col gap-3 overflow-y-auto pr-1">
    {#if pendingSwitch !== undefined}
      <div class="flex items-center gap-3 rounded-md border border-amber-700 bg-amber-950/60 px-3 py-2 text-sm text-amber-100">
        <span class="flex-1">This command has unsaved changes.</span>
        <button
          type="button"
          data-annotate="commands-discard"
          class="rounded-md bg-amber-600 px-2.5 py-1 text-zinc-950"
          onclick={() => switchTo(pendingSwitch ?? null)}>Discard</button
        >
        <button
          type="button"
          data-annotate="commands-keep"
          class="rounded-md px-2.5 py-1 hover:bg-amber-900"
          onclick={() => (pendingSwitch = undefined)}>Keep editing</button
        >
      </div>
    {/if}

    {#if selectedId === null && !creating}
      <p class="text-sm text-zinc-400">Choose a command, or create one with New.</p>
    {:else}
      <div class="flex items-center justify-between gap-3">
        <div class="flex min-w-0 items-center gap-2">
          {#if creating}
            <span class="text-sm text-zinc-400">File name</span>
            <input
              data-annotate="command-id"
              class="{input} w-56 font-mono"
              placeholder={suggestId(form.title) || 'my-command'}
              bind:value={newId}
            />
            <span class="text-sm text-zinc-500">.yaml</span>
          {:else}
            <span class="truncate font-mono text-sm text-zinc-300">commands/{selectedId}.yaml</span>
          {/if}
        </div>
        <div class="flex shrink-0 items-center gap-1 rounded-md bg-zinc-800 p-0.5">
          <button
            type="button"
            data-annotate="command-view-form"
            class="rounded px-2.5 py-0.5 text-sm {view === 'form' ? 'bg-zinc-600 text-zinc-50' : 'text-zinc-400'}"
            onclick={showForm}>Form</button
          >
          <button
            type="button"
            data-annotate="command-view-yaml"
            class="rounded px-2.5 py-0.5 text-sm {view === 'yaml' ? 'bg-zinc-600 text-zinc-50' : 'text-zinc-400'}"
            onclick={showYaml}>YAML</button
          >
        </div>
      </div>

      <Problems {problems} />

      {#if changedWhileEditing}
        <div class="rounded-md border border-amber-700 bg-amber-950/60 px-3 py-2 text-sm text-amber-100">
          The configuration files changed on disk while you were editing. If this file changed, Save will be refused;
          Revert loads it again.
        </div>
      {/if}

      {#if view === 'yaml'}
        <textarea
          data-annotate="command-yaml"
          class="min-h-80 flex-1 rounded-md border border-zinc-600 bg-zinc-950 p-3 font-mono text-sm text-zinc-100 outline-none focus:border-amber-500"
          spellcheck="false"
          bind:value={source}
        ></textarea>
      {:else}
        <div class="flex flex-col gap-4">
          <label class="flex flex-col gap-1">
            <span class={label}>Title</span>
            <input data-annotate="command-title" class={input} bind:value={form.title} />
          </label>

          <div class="flex flex-col gap-1">
            <span class={label}>Action</span>
            <div class="flex flex-wrap gap-1">
              {#each kinds as kind (kind.value)}
                <button
                  type="button"
                  data-annotate="command-kind-{kind.value}"
                  aria-pressed={form.kind === kind.value}
                  class="rounded-md border px-2.5 py-1 text-sm {form.kind === kind.value
                    ? 'border-amber-500 bg-amber-500/25 text-amber-100'
                    : 'border-zinc-600 bg-zinc-800 text-zinc-300 hover:bg-zinc-700'}"
                  onclick={() => (form.kind = kind.value)}>{kind.label}</button
                >
              {/each}
            </div>
          </div>

          {#if form.kind === 'url'}
            <label class="flex flex-col gap-1">
              <span class={label}>URL</span>
              <input data-annotate="command-url" class="{input} font-mono" placeholder="https://" bind:value={form.url} />
            </label>
          {:else if form.kind === 'app'}
            <label class="flex flex-col gap-1">
              <span class={label}>App name or path</span>
              <input data-annotate="command-app" class="{input} font-mono" bind:value={form.app} />
            </label>
            <label class="flex flex-col gap-1">
              <span class={label}>Arguments</span>
              <span class="text-xs text-zinc-500">One argument per line.</span>
              <textarea
                data-annotate="command-args"
                class="{input} h-20 font-mono"
                spellcheck="false"
                bind:value={argsText}
                oninput={(event) => (form.args = argsFromText(event.currentTarget.value))}
              ></textarea>
            </label>
          {:else}
            <label class="flex flex-col gap-1">
              <span class={label}>Command</span>
              <textarea
                data-annotate="command-run"
                class="{input} h-28 font-mono"
                spellcheck="false"
                bind:value={form.run}
              ></textarea>
            </label>
            <label class="flex flex-col gap-1">
              <span class={label}>Working folder</span>
              <input data-annotate="command-cwd" class="{input} font-mono" placeholder="~/" bind:value={form.cwd} />
            </label>
            {#if form.kind === 'script'}
              <div class="flex flex-col gap-1">
                <span class={label}>Environment variables</span>
                <span class="text-xs text-zinc-500">A value may be a 1Password reference such as op://Private/GitHub/token.</span>
                {#each form.env as row, index (index)}
                  <div class="flex gap-2">
                    <input
                      data-annotate="command-env-name-{index}"
                      class="{input} w-48 font-mono"
                      placeholder="NAME"
                      bind:value={row[0]}
                    />
                    <input data-annotate="command-env-value-{index}" class="{input} font-mono" bind:value={row[1]} />
                    <button
                      type="button"
                      data-annotate="command-env-remove-{index}"
                      class="px-2 text-zinc-400 hover:text-zinc-100"
                      aria-label="Remove"
                      onclick={() => (form.env = form.env.filter((_, i) => i !== index))}>×</button
                    >
                  </div>
                {/each}
                <button
                  type="button"
                  data-annotate="command-env-add"
                  class="self-start text-sm text-zinc-400 underline hover:text-zinc-100"
                  onclick={() => (form.env = [...form.env, ['', '']])}>Add variable</button
                >
              </div>
            {/if}
          {/if}

          <div class="flex flex-col gap-1">
            <span class={label}>Keywords</span>
            <span class="text-xs text-zinc-500">Typing one of these exactly puts the command first.</span>
            <TokenInput name="keywords" values={form.keywords} onchange={(values) => (form.keywords = values)} />
          </div>

          <div class="flex flex-col gap-1">
            <span class={label}>Hotkey</span>
            <span class="text-xs text-zinc-500">Runs the command from any app, without the launcher.</span>
            <HotkeyPicker
              name="command"
              value={form.hotkey}
              {os}
              id={creating ? null : selectedId}
              epoch={reloadKey + changedKey}
              optional
              onchange={(value) => (form.hotkey = value)}
            />
          </div>

          <div class="flex flex-col gap-1">
            <span class={label}>Tags</span>
            <TokenInput
              name="tags"
              values={form.tags}
              suggestions={tags}
              onchange={(values) => (form.tags = values)}
            />
          </div>

          <label class="flex items-center gap-2 text-sm text-zinc-300">
            <input type="checkbox" data-annotate="command-pin" bind:checked={form.pin} />
            Pin to the top of the list when nothing is typed
          </label>

          <div class="flex flex-col gap-1">
            <span class={label}>Show on</span>
            <div class="flex items-center gap-4">
              {#each systems as system (system.value)}
                <label class="flex items-center gap-1.5 text-sm text-zinc-300">
                  <input
                    type="checkbox"
                    data-annotate="command-os-{system.value}"
                    checked={form.os.includes(system.value)}
                    onchange={() => toggleOs(system.value)}
                  />
                  {system.label}
                </label>
              {/each}
              <span class="text-xs text-zinc-500">{form.os.length === 0 ? '(none checked: every OS)' : ''}</span>
            </div>
          </div>
        </div>
      {/if}

      <div class="flex flex-wrap items-center gap-2 border-t border-zinc-800 pt-3">
        <button
          type="button"
          data-annotate="command-save"
          class="rounded-md bg-amber-500 px-4 py-1.5 text-sm font-medium text-zinc-950 hover:bg-amber-400 disabled:opacity-40"
          disabled={!dirty || busy}
          onclick={save}>{creating ? 'Create' : 'Save'}</button
        >
        <button
          type="button"
          data-annotate="command-revert"
          class="rounded-md px-3 py-1.5 text-sm text-zinc-300 hover:bg-zinc-800 disabled:opacity-40"
          onclick={revert}>{dirty ? 'Revert' : 'Reload'}</button
        >
        {#if !creating}
          <button
            type="button"
            data-annotate="command-try"
            class="rounded-md px-3 py-1.5 text-sm text-zinc-300 hover:bg-zinc-800 disabled:opacity-40"
            disabled={dirty}
            title={dirty ? 'Save first; Try runs the saved command' : ''}
            onclick={tryIt}>Try</button
          >
          <button
            type="button"
            data-annotate="command-reveal"
            class="rounded-md px-3 py-1.5 text-sm text-zinc-300 hover:bg-zinc-800"
            onclick={() => api.reveal(selectedId)}>{os === 'macos' ? 'Show in Finder' : 'Show file'}</button
          >
          <span class="flex-1"></span>
          <button
            type="button"
            data-annotate="command-delete"
            class="rounded-md px-3 py-1.5 text-sm {confirmDelete
              ? 'bg-red-600 text-white'
              : 'text-red-300 hover:bg-red-950'}"
            onclick={remove}>{confirmDelete ? 'Click again to delete' : 'Delete'}</button
          >
        {/if}
        {#if status}<span class="text-sm text-emerald-300">{status}</span>{/if}
      </div>
    {/if}
  </div>
</div>
