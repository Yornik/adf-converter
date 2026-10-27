--images drop removes attachment links, valid or not: before [report.pdf](adf:attachment/66666666-7777-8888-9999-000000000601) middle [bad](adf:attachment/not-an-id) after.

A paragraph of only attachments disappears with them:

[a.pdf](adf:attachment/66666666-7777-8888-9999-000000000602) [b.pdf](adf:attachment/66666666-7777-8888-9999-000000000603)

Ordinary images go too: ![ordinary](https://example.com/a.png).

Ordinary links stay: [docs](https://example.com/docs).

The text around stays.
