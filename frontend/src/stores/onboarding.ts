import { writable } from 'svelte/store'

/// Whether the first-run onboarding wizard is showing. Set by the layout on load
/// (when the server's onboarding_complete flag is false) and by Display settings
/// (to replay it), cleared when the wizard closes.
export const showOnboarding = writable<boolean>(false)
