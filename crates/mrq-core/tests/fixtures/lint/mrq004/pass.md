# Includes

A region with content is not compared with its file yet:

<!-- mrq:include path="targets/chapter.md" -->
## Chapter (outdated)
<!-- mrq:end -->

A bare marker is expanded by `mrq interpolate`, not stale:

<!-- mrq:include path="targets/chapter.md" -->

An empty region is fine when the expansion may be empty (a file without a body, a `section`, `select` or `lines` filter) or unknown (a missing or remote file):

<!-- mrq:include path="targets/empty.md" -->
<!-- mrq:end -->

<!-- mrq:include path="targets/chapter.md" section="Nowhere" -->
<!-- mrq:end -->

<!-- mrq:include path="targets/missing.md" -->
<!-- mrq:end -->

<!-- mrq:include path="https://example.com/chapter.md" -->
<!-- mrq:end -->
