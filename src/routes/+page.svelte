<script lang="ts">
  import { onMount } from "svelte";
  import { listDevices, setDefault, type AudioDevice } from "$lib/audio";

  let devices = $state<AudioDevice[]>([]);
  let error = $state<string | null>(null);
  let busy = $state<string | null>(null);
  let showInactive = $state(false);

  onMount(load);

  async function load() {
    try {
      devices = await listDevices();
      error = null;
    } catch (e) {
      error = String(e);
    }
  }

  async function select(id: string) {
    busy = id;
    error = null;
    try {
      await setDefault(id);
      await load();
    } catch (e) {
      error = String(e);
    } finally {
      busy = null;
    }
  }

  const isDefault = (device: AudioDevice) =>
    device.is_default_console &&
    device.is_default_multimedia &&
    device.is_default_communications;

  const roles = (device: AudioDevice) => [
    { key: "Console", on: device.is_default_console },
    { key: "Media", on: device.is_default_multimedia },
    { key: "Comm", on: device.is_default_communications },
  ];

  const visible = $derived(
    showInactive
      ? devices
      : devices.filter((d) => d.state === "active" || isDefault(d)),
  );
</script>

<main>
  <header>
    <h1>AudioSwitch</h1>
    <p>Set a device as every Windows audio default at once.</p>
  </header>

  {#if error}<p class="error">{error}</p>{/if}

  <div class="toolbar">
    <label class="filter">
      <input type="checkbox" bind:checked={showInactive} />
      Show inactive devices
    </label>
    <button class="refresh" onclick={load} disabled={busy !== null}>
      Refresh
    </button>
  </div>

  <div class="device-list">
    {#each visible as device (device.id)}
      <div class="device" class:selected={isDefault(device)}>
        <div class="device-main">
          <span class="device-name">{device.name}</span>
          {#if device.state !== "active"}
            <span class="state">{device.state}</span>
          {/if}
          <span class="badges">
            {#each roles(device) as role}
              <span class="badge" class:on={role.on}>{role.key}</span>
            {/each}
          </span>
        </div>
        <button
          onclick={() => select(device.id)}
          disabled={busy !== null}
        >
          {busy === device.id ? "Switching…" : "Set default"}
        </button>
      </div>
    {:else}
      <p class="empty">
        {showInactive
          ? "No playback devices found."
          : "No active playback devices found."}
      </p>
    {/each}
  </div>
</main>

<style>
  :global(body) {
    margin: 0;
    background: #0f1117;
    color: #e6e9ef;
    font-family: "Segoe UI", system-ui, sans-serif;
  }

  main {
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 20px;
    min-height: 100vh;
    box-sizing: border-box;
  }

  header h1 {
    margin: 0 0 4px;
    font-size: 22px;
    letter-spacing: 0.5px;
  }

  header p {
    margin: 0;
    color: #8b93a3;
    font-size: 13px;
  }

  .error {
    color: #ff7a7a;
    font-size: 13px;
    border: 1px solid #5a2a2a;
    background: #2a1515;
    border-radius: 8px;
    padding: 10px 12px;
    margin: 0;
    word-break: break-word;
  }

  .device-list {
    display: flex;
    flex-direction: column;
    gap: 10px;
    flex: 1;
  }

  .device {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    background: #161a23;
    border: 1px solid #232936;
    border-radius: 10px;
    padding: 12px 14px;
    transition: border-color 0.15s;
  }

  .device.selected {
    border-color: #4c8dff;
  }

  .device-main {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }

  .device-name {
    font-size: 14px;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .state {
    color: #d99a3d;
    font-size: 12px;
    text-transform: capitalize;
  }

  .badges {
    display: flex;
    gap: 6px;
  }

  .badge {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.6px;
    color: #5a6272;
    border: 1px solid #2a303d;
    border-radius: 999px;
    padding: 2px 8px;
  }

  .badge.on {
    color: #9cc1ff;
    border-color: #2c4a80;
    background: #16233a;
  }

  button {
    background: #232936;
    color: #e6e9ef;
    border: 1px solid #303748;
    border-radius: 8px;
    padding: 8px 14px;
    font-size: 13px;
    cursor: pointer;
    white-space: nowrap;
  }

  button:hover:not(:disabled) {
    background: #2b3242;
    border-color: #4c8dff;
  }

  button:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .filter {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    color: #8b93a3;
    cursor: pointer;
    user-select: none;
  }

  .filter input {
    accent-color: #4c8dff;
    width: 15px;
    height: 15px;
    cursor: pointer;
  }

  .refresh {
    align-self: auto;
  }

  .empty {
    color: #8b93a3;
    font-size: 13px;
  }
</style>
