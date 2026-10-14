<script lang="ts">
  import { t as translateText } from '../i18n';
  import { onMount, type Snippet } from 'svelte';
  let { title, onclose, children }: { title: string; onclose: () => void; children: Snippet } = $props();
  let dialog: HTMLDialogElement;
  onMount(() => dialog.showModal());
</script>

<dialog bind:this={dialog} oncancel={e => { e.preventDefault(); onclose(); }} aria-label={title}>
  <header class="modal-header"><h2>{title}</h2><button class="icon-button" aria-label={$translateText("Close dialog")} onclick={onclose}>×</button></header>
  {@render children()}
</dialog>
