## Fixes

- Fixed Redis key types changing with value content: a plain STRING that contains JSON text still shows STRING, and only real JSON keys (ReJSON) show JSON.
- Fixed long Redis JSON values being cut off in the read-only view: content taller than the panel now scrolls vertically, so the full value is visible without entering edit mode.
- Fixed the Redis JSON view showing the previous key's value when the current value fails to parse; it now falls back to the current key's raw text.
- Fixed choosing JSON when creating a key silently storing a plain string on servers without the RedisJSON module. Creation now fails with a clear error instead of producing a key that shows STRING.
- Fixed the new-key and value-edit inputs only focusing when the text area itself is clicked; clicking anywhere inside the box now focuses the input.

## Improvements

- Switching a String value to the JSON format now renders a read-only view with syntax highlighting, line numbers and indentation; invalid JSON falls back to the raw text.
