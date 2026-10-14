# JSON-RPC 2.0 Specification (jsonrpc.org/specification)

Governs `ledger-mcp`'s wire protocol (MCP is JSON-RPC 2.0). Fetched
2026-10-14.

**5 Response object**

> When a rpc call is made, the Server MUST reply with a Response, except
> for in the case of Notifications. [...]
>
> result: This member is REQUIRED on success. This member MUST NOT exist
> if there was an error invoking the method. [...]
>
> error: This member is REQUIRED on error. This member MUST NOT exist if
> there was no error triggered during invocation. [...]
>
> id: This member is REQUIRED. It MUST be the same as the value of the id
> member in the Request Object. If there was an error in detecting the id
> in the Request object (e.g. Parse error/Invalid Request), it MUST be
> Null.
>
> Either the result member or error member MUST be included, but both
> members MUST NOT be included.

**5.1 Error object**

> code: A Number that indicates the error type that occurred. This MUST be
> an integer. [...]
>
> | code | message | meaning |
> |---|---|---|
> | -32700 | Parse error | Invalid JSON was received by the server. |
> | -32600 | Invalid Request | The JSON sent is not a valid Request object. |
> | -32601 | Method not found | The method does not exist / is not available. |
> | -32602 | Invalid params | Invalid method parameter(s). |
> | -32603 | Internal error | Internal JSON-RPC error. |
