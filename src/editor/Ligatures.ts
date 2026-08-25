import { Extension, textInputRule } from "@tiptap/core";

export const rightArrow = textInputRule({
  find: /->$/,
  replace: "→",
});

export const leftArrow = textInputRule({
  find: /<-$/,
  replace: "←",
});

export const doubleRightArrow = textInputRule({
  find: /=>$/,
  replace: "⇒",
});

export const doubleLeftArrow = textInputRule({
  find: /<=$/,
  replace: "⇐",
});

export const bidirectionalArrow = textInputRule({
  find: /(?:<->|←>)$/,
  replace: "↔",
});

export const doubleBidirectionalArrow = textInputRule({
  find: /(?:<=>|⇐>)$/,
  replace: "⇔",
});

export const emDash = textInputRule({
  find: /--$/,
  replace: "—",
});

export const Ligatures = Extension.create({
  name: "ligatures",

  addInputRules() {
    return [
      bidirectionalArrow,
      doubleBidirectionalArrow,
      rightArrow,
      leftArrow,
      doubleRightArrow,
      doubleLeftArrow,
      emDash,
    ];
  },
});
