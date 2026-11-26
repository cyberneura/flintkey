<script lang="ts">
  /** The icon a row shows when it has no picture of its own: one per command type. */
  let { kind }: { kind: string } = $props();
</script>

<div class="flex size-8 items-center justify-center rounded-lg bg-zinc-800 text-zinc-400" data-annotate="type-icon-{kind}">
  <svg
    viewBox="0 0 24 24"
    class="size-5"
    fill="none"
    stroke="currentColor"
    stroke-width="1.75"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    {#if kind === 'url'}
      <!-- A globe. -->
      <circle cx="12" cy="12" r="9" />
      <path d="M3 12h18" />
      <path d="M12 3c2.5 2.7 3.75 5.7 3.75 9S14.5 18.3 12 21c-2.5-2.7-3.75-5.7-3.75-9S9.5 5.7 12 3z" />
    {:else if kind === 'app'}
      <!-- Four tiles. -->
      <rect x="4" y="4" width="7" height="7" rx="1.5" />
      <rect x="13" y="4" width="7" height="7" rx="1.5" />
      <rect x="4" y="13" width="7" height="7" rx="1.5" />
      <rect x="13" y="13" width="7" height="7" rx="1.5" />
    {:else if kind === 'terminal'}
      <!-- A window with a prompt. -->
      <rect x="3" y="4.5" width="18" height="15" rx="2" />
      <path d="M7 10l3 2.5L7 15" />
      <path d="M12.5 15H17" />
    {:else}
      <!-- Code brackets, for scripts. -->
      <path d="M8.5 7L4 12l4.5 5" />
      <path d="M15.5 7L20 12l-4.5 5" />
      <path d="M13.5 5l-3 14" />
    {/if}
  </svg>
</div>
