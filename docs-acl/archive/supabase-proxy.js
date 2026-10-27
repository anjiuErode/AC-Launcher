const UPSTREAM = "https://hehzfiafjqthetdxekxf.supabase.co";

export default {
	async fetch(request) {
		const incoming = new URL(request.url);
		const target = UPSTREAM + incoming.pathname + incoming.search;

		const headers = new Headers(request.headers);
		headers.set("Host", new URL(UPSTREAM).host);
		headers.delete("cf-connecting-ip");
		headers.delete("cf-ray");

		const hasBody = !["GET", "HEAD"].includes(request.method);

		const upstreamResponse = await fetch(target, {
			method: request.method,
			headers,
			body: hasBody ? request.body : undefined,
			redirect: "manual",
		});

		const responseHeaders = new Headers(upstreamResponse.headers);
		responseHeaders.set("access-control-allow-origin", "*");
		responseHeaders.delete("content-security-policy");

		return new Response(upstreamResponse.body, {
			status: upstreamResponse.status,
			statusText: upstreamResponse.statusText,
			headers: responseHeaders,
		});
	},
};
