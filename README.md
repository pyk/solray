# hawk

Foundry project inspector.

- Given a Foundry project, it can resolve all deployable contracts, libraries and
  abstract contracts.
- Given a `Contract`, it can resolve all external/public functions, useful for audit.
- Given `Contract::function`, it can resolve the complete source code of a function,
  better than grepping manually.
- Given `Contract::function`, it can resolve the complete call graph of a function,
  better than grepping manually.
