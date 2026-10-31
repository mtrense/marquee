A tight list:

* a
* b
    * c
    * d
* e

Loose, by a blank line between items:

* a

* b
* c

Loose, by a blank line inside an item:

1. a

   b
2. c

Tight, with a loose list inside:

- a
  - b

  - c
- d

Loose items around tight content:

- a
  ```
  code
  ```
  > quote

- b
