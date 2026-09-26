# Acme App Overview

Acme App is a small e-commerce storefront. This document is project knowledge
consumed by the Context Resolver — it is not code, and it is not a skill; it
is background that helps an agent (or a human) understand the product.

## Checkout

The checkout flow lets a signed-in customer review their cart and submit
payment through the `CheckoutButton` component. Payment is processed by the
`payment-flow` capability, and the result is shown via `CheckoutSummary`.

Checkout interactions must follow the `checkout-ux` skill and the
`checkout-button` behavior specification, and must satisfy the project's
`accessibility` skill.
