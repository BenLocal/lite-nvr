import { ref } from 'vue'
import { getSettings, saveSettings, type PlayerBackend } from '../api/settings'

// Module-level singletons: every player shares one reactive preference, loaded
// from the backend at most once. Defaults to mpegts.js until settings arrive.
const playerBackend = ref<PlayerBackend>('mpegts')
const loaded = ref(false)
let loadPromise: Promise<void> | null = null

/** Load the persisted preference once; keeps the mpegts default on failure. */
export function ensurePlayerPreference(): Promise<void> {
  if (!loadPromise) {
    loadPromise = getSettings()
      .then((s) => {
        if (s?.player) {
          playerBackend.value = s.player
        }
      })
      .catch(() => {
        // Keep the default (mpegts) if settings can't be fetched.
      })
      .finally(() => {
        loaded.value = true
      })
  }
  return loadPromise
}

/** Resolve the current player backend, loading it first if needed. */
export async function resolvePlayerBackend(): Promise<PlayerBackend> {
  await ensurePlayerPreference()
  return playerBackend.value
}

/** Persist a new backend and update the shared reactive value. */
export async function savePlayerBackend(backend: PlayerBackend): Promise<void> {
  const saved = await saveSettings({ player: backend })
  playerBackend.value = saved?.player ?? backend
  loaded.value = true
}

export function usePlayerPreference() {
  void ensurePlayerPreference()
  return { playerBackend, loaded }
}
