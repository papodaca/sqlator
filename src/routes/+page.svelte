<script lang="ts">
  import TabbedEditor from "$lib/components/TabbedEditor.svelte";

  // TabbedEditor needs a way to open the connection form from the sidebar.
  // We bubble this up through a custom event on the window.
  function openConnectionForm() {
    window.dispatchEvent(new CustomEvent("sqlator:open-connection-form"));
  }
</script>

<TabbedEditor onopenconnectionform={openConnectionForm} />
