# No URLs as inline links

[https://example.com](https://example.com) is reported, [text](https://example.com) is not,
nor is [https://example.com](https://example.com "Title") or [relative](relative).

A full reference [https://example.com][url] becomes an autolink.

[url]: https://example.com

<!-- markdownlint-configure-file { "MD054": { "url_inline": false, "full": false } } -->
