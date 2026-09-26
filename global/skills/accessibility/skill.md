---
id: accessibility
name: Accessibility
version: 1.0.0
scope: global
description: Baseline accessibility rules applicable to any interactive UI element
tags:
  - accessibility
  - ux
  - a11y
---

# Accessibility

Baseline rules for any agent implementing interactive UI, across all
projects.

## Rules

- Every interactive control must be reachable and operable via keyboard
  alone.
- State changes that are conveyed visually (loading, success, error) must
  also be conveyed to assistive technology, e.g. via `aria-live` or
  equivalent.
- Color must never be the only signal for state; pair it with text or an
  icon with a text alternative.
- Focus must move predictably: never trap focus, and return focus to a
  sensible element after a modal or transient state closes.
