<script lang="ts">
  import { untrack } from 'svelte';
  import { api, problemsOf, type SettingsForm, type SettingsPage } from './api';
  import HotkeyPicker from './HotkeyPicker.svelte';
  import Problems from './Problems.svelte';

  let { reloadKey, changedKey }: { reloadKey: number; changedKey: number } = $props();
  /** The files changed on disk while there were unsaved edits here. */
  let changedWhileEditing = $state(false);

  let page = $state<SettingsPage | null>(null);
  let form = $state<SettingsForm>({ hotkey: '', display: 'cursor', terminal: null });
  let source = $state('');
  let editingSource = $state(false);
  let problems = $state<string[]>([]);
  let status = $state('');
  let saving = $state(false);

  const dirty = $derived(
    page !== null &&
      (editingSource ? source !== page.source : JSON.stringify(form) !== JSON.stringify(page.form)),
  );

  function show(loaded: SettingsPage) {
    changedWhileEditing = false;
    page = loaded;
    form = { ...loaded.form };
    source = loaded.source;
    problems = loaded.problems;
    // The form would show the last settings that could be read, not what the file says.
    if (!loaded.readable) {
      editingSource = true;
    }
  }

  async function load() {
    try {
      show(await api.settingsPage());
    } catch (error) {
      problems = problemsOf(error);
    }
  }

  // Reload whenever the window is shown again or the files change on disk. Unsaved edits are kept (Revert loads
  // the file again); a change on disk under them is pointed out.
  $effect(() => {
    void reloadKey;
    untrack(() => {
      if (!dirty) load();
    });
  });

  $effect(() => {
    void changedKey;
    untrack(() => {
      if (changedKey === 0) return;
      if (dirty) {
        changedWhileEditing = true;
      } else {
        load();
      }
    });
  });


  async function save() {
    saving = true;
    status = '';
    try {
      const base = page?.source ?? '';
      show(editingSource ? await api.settingsSaveSource(source, base) : await api.settingsSave(form, base));
      status = 'Saved.';
    } catch (error) {
      problems = problemsOf(error);
    } finally {
      saving = false;
    }
  }

  // Loads the file again, so a file changed outside the window can be picked up.
  async function revert() {
    status = '';
    await load();
  }

  const terminals = [
    { value: '', label: 'OS default' },
    { value: 'terminal', label: 'Terminal.app (macOS)' },
    { value: 'windows-terminal', label: 'Windows Terminal (Windows)' },
  ];
</script>

{#if page}
  <div class="flex max-w-3xl flex-col gap-6">
    <div class="flex items-center justify-between gap-4">
      <h1 class="text-lg font-semibold text-zinc-100">General</h1>
      <div class="flex items-center gap-2">
        <button
          type="button"
          data-annotate="general-toggle-yaml"
          class="rounded-md px-2.5 py-1 text-sm text-zinc-400 hover:text-zinc-100"
          onclick={() => {
            if (!dirty) editingSource = !editingSource;
          }}
          disabled={dirty || !page.readable}
          title={dirty ? 'Save or revert first' : !page.readable ? 'settings.yaml has problems; fix it as YAML' : ''}
        >
          {editingSource ? 'Edit as form' : 'Edit as YAML'}
        </button>
      </div>
    </div>

    <Problems {problems} />

    {#if changedWhileEditing}
      <div class="rounded-md border border-amber-700 bg-amber-950/60 px-3 py-2 text-sm text-amber-100">
        The configuration files changed on disk while you were editing. If this file changed, Save will be refused;
        Revert loads it again.
      </div>
    {/if}

    {#if editingSource}
      <textarea
        data-annotate="general-yaml"
        class="h-72 w-full rounded-md border border-zinc-600 bg-zinc-950 p-3 font-mono text-sm text-zinc-100 outline-none focus:border-amber-500"
        spellcheck="false"
        bind:value={source}
      ></textarea>
    {:else}
      <section class="flex flex-col gap-2">
        <h2 class="text-sm font-medium text-zinc-200">Launcher hotkey</h2>
        <p class="text-sm text-zinc-400">The key combination that shows and hides the launcher from any app.</p>
        <HotkeyPicker
          name="launcher"
          value={form.hotkey}
          os={page.os}
          launcher
          epoch={reloadKey + changedKey}
          onchange={(value) => (form.hotkey = value)}
        />
      </section>

      <section class="flex flex-col gap-2">
        <h2 class="text-sm font-medium text-zinc-200">Show the launcher on</h2>
        <div class="flex flex-col gap-1.5">
          <label class="flex items-center gap-2 text-sm text-zinc-300">
            <input type="radio" data-annotate="general-display-cursor" bind:group={form.display} value="cursor" />
            The display with the mouse pointer
          </label>
          <label class="flex items-center gap-2 text-sm text-zinc-300">
            <input type="radio" data-annotate="general-display-primary" bind:group={form.display} value="primary" />
            The main display
          </label>
        </div>
      </section>

      <section class="flex flex-col gap-2">
        <h2 class="text-sm font-medium text-zinc-200">Terminal for <code>type: terminal</code></h2>
        <select
          data-annotate="general-terminal"
          class="w-72 rounded-md border border-zinc-600 bg-zinc-800 px-2 py-1 text-sm text-zinc-100"
          value={form.terminal ?? ''}
          onchange={(event) => (form.terminal = event.currentTarget.value || null)}
        >
          {#each terminals as terminal (terminal.value)}
            <option value={terminal.value}>{terminal.label}</option>
          {/each}
        </select>
      </section>
    {/if}

    <div class="flex items-center gap-3">
      <button
        type="button"
        data-annotate="general-save"
        class="rounded-md bg-amber-500 px-4 py-1.5 text-sm font-medium text-zinc-950 hover:bg-amber-400 disabled:opacity-40"
        disabled={!dirty || saving || (!editingSource && form.hotkey === '')}
        onclick={save}
      >
        Save
      </button>
      <button
        type="button"
        data-annotate="general-revert"
        class="rounded-md px-3 py-1.5 text-sm text-zinc-300 hover:bg-zinc-800 disabled:opacity-40"
        onclick={revert}
      >
        Revert
      </button>
      {#if status}<span class="text-sm text-emerald-300">{status}</span>{/if}
    </div>

    <section class="flex flex-col gap-1 border-t border-zinc-800 pt-4 text-sm text-zinc-400">
      <span>Settings are saved to <code class="text-zinc-300">{page.config_dir}/settings.yaml</code>.</span>
      <span>
        <button
          type="button"
          data-annotate="general-reveal"
          class="underline hover:text-zinc-100"
          onclick={() => api.reveal(null)}
        >
          Open the configuration folder
        </button>
      </span>
    </section>
  </div>
{:else}
  <Problems {problems} />
{/if}
