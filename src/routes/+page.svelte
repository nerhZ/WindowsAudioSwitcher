<script lang="ts">
  import { onMount } from "svelte";
  import { listDevices, setDefault, type AudioDevice, type Role } from "$lib/audio";
  import RoleBadge from "$lib/components/RoleBadge.svelte";

  let devices = $state<AudioDevice[]>([]);
  let error = $state<string | null>(null);
  let busy = $state<string | null>(null);
  let showInactive = $state(false);
  let rolesOn = $state<Record<string, boolean>>({ console: true, media: true, comm: true });

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
      await setDefault(id, selectedRoles);
      await load();
    } catch (e) {
      error = String(e);
    } finally {
      busy = null;
    }
  }

  const toggleRole = (key: string) => {
    rolesOn[key] = !rolesOn[key];
  };

  const roleOptions = [
    { key: "console", label: "Console" },
    { key: "media", label: "Media" },
    { key: "comm", label: "Comm" },
  ];

  const selectedRoles = $derived<Role[]>(
    roleOptions.flatMap((r) =>
      rolesOn[r.key]
        ? [r.key === "console" ? "console" : r.key === "media" ? "multimedia" : "communications"]
        : [],
    ),
  );

  const isDefault = (device: AudioDevice) =>
    device.is_default_console &&
    device.is_default_multimedia &&
    device.is_default_communications;

  const visible = $derived(
    showInactive
      ? devices
      : devices.filter((d) => d.state === "active" || isDefault(d)),
  );
</script>

<main class="flex min-h-screen flex-col gap-4 p-5">
  <header>
    <h1 class="text-2xl font-bold tracking-tight">AudioSwitch</h1>
    <p class="mt-1 text-sm text-base-content/60">
      Set a device as every Windows audio default at once.
    </p>
  </header>

  {#if error}
    <div role="alert" class="alert alert-error py-2.5">
      <span class="text-sm break-all">{error}</span>
    </div>
  {/if}

  <div class="flex items-center justify-between gap-4">
    <label class="flex cursor-pointer select-none items-center gap-2 text-sm text-base-content/80">
      <input
        type="checkbox"
        class="toggle toggle-primary toggle-sm"
        bind:checked={showInactive}
      />
      Show inactive devices
    </label>
    <button class="btn btn-ghost btn-sm" onclick={load} disabled={busy !== null}>
      Refresh
    </button>
  </div>

  <div class="flex flex-wrap items-center gap-1.5">
    {#each roleOptions as role}
      <RoleBadge
        label={role.label}
        on={rolesOn[role.key]}
        onclick={() => toggleRole(role.key)}
      />
    {/each}
    {#if selectedRoles.length === 0}
      <span class="text-xs text-base-content/50">Select at least one role to switch.</span>
    {/if}
  </div>

  <div class="flex flex-1 flex-col gap-2.5">
    {#each visible as device (device.id)}
      <div
        class="card flex-row items-center justify-between gap-4 rounded-2xl bg-base-200 p-4 {isDefault(device) ? 'ring-2 ring-primary' : ''}"
      >
        <div class="flex min-w-0 flex-col gap-1.5">
          <div class="flex flex-wrap items-center gap-2">
            <span class="truncate text-sm font-semibold">{device.name}</span>
            {#if isDefault(device)}
              <span class="badge badge-success badge-sm">Default</span>
            {/if}
            {#if device.state !== "active"}
              <span class="badge badge-warning badge-sm capitalize">{device.state}</span>
            {/if}
          </div>
        </div>
        <button
          class="btn btn-primary btn-sm"
          onclick={() => select(device.id)}
          disabled={busy !== null || selectedRoles.length === 0}
        >
          {#if busy === device.id}
            <span class="loading loading-spinner loading-xs"></span>
            Switching
          {:else}
            Set default
          {/if}
        </button>
      </div>
    {:else}
      <p class="text-sm text-base-content/60">
        {showInactive
          ? "No playback devices found."
          : "No active playback devices found."}
      </p>
    {/each}
  </div>
</main>
