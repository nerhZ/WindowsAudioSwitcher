<script lang="ts">
  let { label, on, onclick }: {
    label: string;
    on: boolean;
    onclick?: () => void;
  } = $props();
</script>

{#if onclick}
  <button
    type="button"
    class="badge badge-sm cursor-pointer select-none transition-colors {on ? 'badge-primary' : 'badge-outline'}"
    onclick={onclick}
  >
    {label}
  </button>
{:else}
  <span class="badge badge-sm {on ? 'badge-primary' : 'badge-outline'}">{label}</span>
{/if}
