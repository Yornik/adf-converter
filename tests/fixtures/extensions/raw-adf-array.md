```adf
[
  {
    "type": "table",
    "attrs": {
      "isNumberColumnEnabled": false,
      "layout": "default"
    },
    "content": [
      {
        "type": "tableRow",
        "content": [
          {
            "type": "tableHeader",
            "content": [
              {
                "type": "paragraph",
                "content": [
                  {
                    "type": "text",
                    "text": "Row header"
                  }
                ]
              }
            ]
          },
          {
            "type": "tableCell",
            "content": [
              {
                "type": "paragraph",
                "content": [
                  {
                    "type": "text",
                    "text": "value"
                  }
                ]
              }
            ]
          }
        ]
      },
      {
        "type": "tableRow",
        "content": [
          {
            "type": "tableCell",
            "attrs": {
              "colspan": 2
            },
            "content": [
              {
                "type": "paragraph",
                "content": [
                  {
                    "type": "text",
                    "text": "spans both columns"
                  }
                ]
              }
            ]
          }
        ]
      }
    ]
  },
  {
    "type": "paragraph",
    "content": [
      {
        "type": "text",
        "text": "coloured",
        "marks": [
          {
            "type": "textColor",
            "attrs": {
              "color": "#ff5630"
            }
          }
        ]
      }
    ]
  }
]
```
