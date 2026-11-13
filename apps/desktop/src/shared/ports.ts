/** The ports Hoista listens on, as the engine sets them. */

/** MCP over HTTP on this computer, for web agents reached through a tunnel. */
export const MCP_HTTP_PORT = 8788;
export const MCP_HTTP_ADDRESS = `http://127.0.0.1:${MCP_HTTP_PORT}`;

/** A remote's own port, behind its HTTPS proxy or tunnel; on this computer in Docker, its address. */
export const REMOTE_PORT = 8787;
export const LOCAL_REMOTE_ADDRESS = `http://localhost:${REMOTE_PORT}`;
