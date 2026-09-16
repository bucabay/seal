/**
 * Props for any field holding a credential or a reference name.
 *
 * A browser will happily capitalise the first letter, swap a straight quote for
 * a curly one, or "correct" a word inside a key — and a secret that has been
 * autocorrected is simply wrong, with nothing on screen to say so. Password
 * managers offering to fill or save the field are a different kind of wrong.
 *
 * Applied to every input that carries a value or a reference.
 */
export const rawInput = {
  autoCorrect: "off",
  autoCapitalize: "off",
  spellCheck: false,
  autoComplete: "off",
  // Grammarly and similar extensions inject into fields regardless of the
  // attributes above.
  "data-gramm": "false",
  "data-gramm_editor": "false",
  "data-enable-grammarly": "false",
} as const;
