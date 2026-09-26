---
id: checkout-ux
name: Checkout UX
version: 1.0.0
scope: project
description: Rules for implementing checkout interactions
tags:
  - checkout
  - ux
dependsOn:
  - accessibility
---

# Checkout UX

Instructions for any agent implementing or modifying the checkout flow.

## Rules

- The checkout button must never allow a double submit while a payment is
  in flight. Disable it (do not hide it) once submission starts.
- User-entered form data must be preserved if payment fails, so the customer
  can retry without re-entering everything.
- Success and error states must be visually distinct and announced to
  assistive technology (see the `accessibility` skill).
- Follow the `checkout-button` behavior specification for the exact state
  machine (`idle -> loading -> success | error`).

## Notes for implementers

- Treat the checkout button as a controlled component driven by the
  `checkout-button` behavior spec's states, not by ad-hoc booleans.
- Payment submission is handled by the `payment-flow` capability; this skill
  only governs the UI-facing behavior around it.
