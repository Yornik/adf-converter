A paragraph of only attachment links becomes a media group:

[report.pdf](adf:attachment/66666666-7777-8888-9999-000000000000) [log.txt](adf:attachment/66666666-7777-8888-9999-000000000001)

A single attachment is a group of one, and a collection is optional:

[trace.zip](adf:attachment/66666666-7777-8888-9999-000000000002?collection=contentId-123456)

A line break between the links keeps the group, and an empty collection is what Jira writes:

[first.pdf](adf:attachment/66666666-7777-8888-9999-000000000003)
[second.pdf](adf:attachment/66666666-7777-8888-9999-000000000004?collection=)

The scheme and the kind are not case sensitive, and the id keeps its case:

[Upper.PDF](ADF:Attachment/AAAAAAAA-BBBB-2222-CCCC-DDDDEEEEFFFF)

Text around the link makes it an inline file: see [trace.zip](adf:attachment/66666666-7777-8888-9999-000000000002?collection=contentId-123456) for details, and [log.txt](adf:attachment/66666666-7777-8888-9999-000000000001) as well.

Links with nothing between them stay inline: [a.txt](adf:attachment/66666666-7777-8888-9999-000000000005)[b.txt](adf:attachment/66666666-7777-8888-9999-000000000006)

A link title is not part of the node: [titled.pdf](adf:attachment/66666666-7777-8888-9999-000000000007 "Not kept")
