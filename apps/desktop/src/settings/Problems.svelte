<script lang="ts">
  let { problems }: { problems: string[] } = $props();
</script>

{#if problems.length > 0}
  <ul class="flex flex-col gap-1 rounded-md border border-red-900/60 bg-red-950/50 px-3 py-2 text-sm text-red-200">
    {#each problems as problem (problem)}
      <li class="break-words whitespace-pre-wrap">{problem}</li>
    {/each}
  </ul>
{/if}
