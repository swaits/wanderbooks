# Model Context Protocol: tools (specification 2025-06-18)

Governs `ledger-mcp`'s tool results and errors. Source:
https://github.com/modelcontextprotocol/modelcontextprotocol,
docs/specification/2025-06-18/server/tools.mdx (MIT). Fetched 2026-10-14.

**Structured Content**

> **Structured** content is returned as a JSON object in the
> `structuredContent` field of a result.
>
> For backwards compatibility, a tool that returns structured content
> SHOULD also return the serialized JSON in a TextContent block.

**Output Schema**

> Tools may also provide an output schema for validation of structured
> results. If an output schema is provided:
>
> - Servers **MUST** provide structured results that conform to this schema.
> - Clients **SHOULD** validate structured results against this schema.

**Error Handling**

> Tools use two error reporting mechanisms:
>
> 1. **Protocol Errors**: Standard JSON-RPC errors for issues like:
>    - Unknown tools
>    - Invalid arguments
>    - Server errors
>
> 2. **Tool Execution Errors**: Reported in tool results with `isError: true`:
>    - API failures
>    - Invalid input data
>    - Business logic errors

**Security Considerations**

> 1. Servers **MUST**:
>    - Validate all tool inputs
>    - Implement proper access controls
>    - Rate limit tool invocations
>    - Sanitize tool outputs

(A local stdio server for one bookkeeper: access control is the file
system's, and rate limiting has no one to limit; wanderbooks takes
"validate all tool inputs" and "sanitize tool outputs" as binding.)
