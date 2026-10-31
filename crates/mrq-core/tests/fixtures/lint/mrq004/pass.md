# Includes

A region that is up to date:

<!-- mrq:include path="targets/chapter.md" -->

## Chapter

Included by the MRQ004 fixtures.

<!-- mrq:end -->

A bare marker is expanded by `mrq interpolate`, not stale:

<!-- mrq:include path="targets/chapter.md" -->

An empty region is fine when the included content is empty:

<!-- mrq:include path="targets/empty.md" -->
<!-- mrq:end -->

Regions whose include fails (`mrq validate` and `mrq interpolate` report why) or is remote are not checked:

<!-- mrq:include path="targets/chapter.md" section="Nowhere" -->
<!-- mrq:end -->

<!-- mrq:include path="targets/missing.md" -->
<!-- mrq:end -->

<!-- mrq:include path="https://example.com/chapter.md" -->
<!-- mrq:end -->
