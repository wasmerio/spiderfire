declare type HeadersInit = Headers | [string, string][] | Record<string, string>;

declare class Headers implements Iterable<[string, string]> {
	constructor(init?: HeadersInit);

	append(name: string, value: string): void;

	delete(name: string): void;

	get(name: string): string | null;

	getSetCookie(): string[];

	has(name: string): boolean;

	set(name: string, value: string): void;

	[Symbol.iterator](): Iterator<[string, string]>;
}

declare type BodyInit = BufferSource | Blob | URLSearchParams | string;

declare type RequestInfo = Request | string;

declare type RequestDestination =
	""
	| "audio"
	| "audioworklet"
	| "document"
	| "embed"
	| "font"
	| "frame"
	| "iframe"
	| "image"
	| "manifest"
	| "object"
	| "paintworklet"
	| "report"
	| "script"
	| "sharedworker"
	| "style"
	| "track"
	| "video"
	| "worker"
	| "xslt";

declare type ReferrerPolicy =
	""
	| "no-referrer"
	| "no-referrer-when-downgrade"
	| "same-origin"
	| "origin"
	| "strict-origin"
	| "origin-when-cross-origin"
	| "strict-origin-when-cross-origin"
	| "unsafe-url";

declare type RequestMode = "navigate" | "same-origin" | "no-cors" | "cors";

declare type RequestCredentials = "omit" | "same-origin" | "include";

declare type RequestCache = "default" | "no-store" | "reload" | "no-cache" | "force-cache" | "only-if-cached";

declare type RequestRedirect = "follow" | "error" | "manual";

declare type RequestDuplex = "half";

declare type RequestPriority = "high" | "low" | "auto";

declare interface RequestInit {
	method?: string;
	headers?: HeadersInit;
	body?: BodyInit;

	referrer?: string;
	referrerPolicy?: ReferrerPolicy;

	mode?: RequestMode;
	credentials?: RequestCredentials;
	cache?: RequestCache;
	redirect?: RequestRedirect;

	integrity?: string;
	keepalive?: boolean;
	signal?: AbortSignal;

	duplex?: RequestDuplex;
	priority?: RequestPriority;
	window?: null;
}

declare class Request {
	constructor(input: RequestInfo, init?: RequestInit);

	get method(): string;

	get url(): string;

	get headers(): Headers;

	get destination(): RequestDestination;

	get referrer(): string;

	get referrerPolicy(): ReferrerPolicy;

	get mode(): RequestMode;

	get credentials(): RequestCredentials;

	get cache(): RequestCache;

	get redirect(): RequestRedirect;

	get integrity(): string;

	get keepalive(): boolean;

	get isReloadNavigation(): string;

	get isHistoryNavigation(): string;

	get signal(): AbortSignal;

	get duplex(): RequestDuplex;
}

declare interface ResponseInit {
	status?: number;
	statusText?: string;
	headers?: HeadersInit;
}

declare type ResponseType = "basic" | "cors" | "default" | "error" | "opaque" | "opaqueredirect";

declare class Response {
	constructor(body?: BodyInit, init?: ResponseInit);

	get type(): ResponseType;

	get url(): string;

	get redirected(): boolean;

	get status(): number;

	get ok(): boolean;

	get statusText(): string;

	get headers(): Headers;

	get bodyUsed(): boolean;

	arrayBuffer(): Promise<ArrayBuffer>;

	text(): Promise<string>;
}

declare function fetch(input: RequestInfo, init?: RequestInit): Promise<Response>;
