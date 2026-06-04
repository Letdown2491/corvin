<script lang="ts">
  // A small "?" affordance that opens the embedded help at a specific topic.
  // Opens in a new tab so it never discards an in-progress modal.
  let { anchor, title = 'Open help' }: { anchor: string; title?: string } = $props()
</script>

<a
  class="help-link"
  href="/help#{anchor}"
  target="_blank"
  rel="noopener noreferrer"
  {title}
  aria-label={title}
>?</a>

<style>
  .help-link {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 15px;
    height: 15px;
    border-radius: 50%;
    border: 1px solid var(--border);
    color: var(--text-muted);
    font-size: 0.7rem;
    font-weight: 700;
    line-height: 1;
    text-decoration: none;
    cursor: help;
    vertical-align: middle;
    flex-shrink: 0;
  }
  .help-link:hover { border-color: var(--accent); color: var(--accent); }
</style>
