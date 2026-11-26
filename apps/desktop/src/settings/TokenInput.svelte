<script lang="ts">
  interface Props {
    values: string[];
    onchange: (values: string[]) => void;
    placeholder?: string;
    /** Values to suggest while typing. */
    suggestions?: string[];
    name: string;
  }

  let { values, onchange, placeholder = '', suggestions = [], name }: Props = $props();
  let draft = $state('');
  const listId = $derived(`suggestions-${name}`);

  function add() {
    const value = draft.trim();
    draft = '';
    if (value !== '' && !values.includes(value)) {
      onchange([...values, value]);
    }
  }

  function remove(index: number) {
    onchange(values.filter((_, i) => i !== index));
  }

  function onKeydown(event: KeyboardEvent) {
    if (event.isComposing || event.keyCode === 229) {
      return;
    }
    if (event.key === 'Enter' || event.key === ',') {
      event.preventDefault();
      add();
    } else if (event.key === 'Backspace' && draft === '' && values.length > 0) {
      remove(values.length - 1);
    }
  }
</script>

<div
  class="flex flex-wrap items-center gap-1.5 rounded-md border border-zinc-600 bg-zinc-800 px-2 py-1 focus-within:border-amber-500"
>
  {#each values as value, index (index)}
    <span class="flex items-center gap-1 rounded bg-zinc-700 px-1.5 py-0.5 text-sm text-zinc-100">
      {value}
      <button
        type="button"
        data-annotate="tokens-{name}-remove-{index}"
        aria-label="Remove {value}"
        class="text-zinc-400 hover:text-zinc-100"
        onclick={() => remove(index)}>×</button
      >
    </span>
  {/each}
  <input
    data-annotate="tokens-{name}-input"
    class="min-w-24 flex-1 bg-transparent py-0.5 text-sm text-zinc-100 outline-none placeholder:text-zinc-500"
    {placeholder}
    list={suggestions.length > 0 ? listId : undefined}
    bind:value={draft}
    onkeydown={onKeydown}
    onblur={add}
  />
  {#if suggestions.length > 0}
    <datalist id={listId}>
      {#each suggestions as suggestion (suggestion)}
        <option value={suggestion}></option>
      {/each}
    </datalist>
  {/if}
</div>
