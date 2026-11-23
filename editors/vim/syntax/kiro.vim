if exists('b:current_syntax')
  finish
endif

syntax keyword kiroKeyword fn struct enum match case if elif else while for in try catch return break continue pass import export as and or not contains
syntax keyword kiroConstant true false null
syntax keyword kiroBuiltin print input len lines range filter map reduce any all sort unique parallel retry exit
syntax match kiroNumber /\<\d\+\(\.\d\+\)\?\([eE][+-]\?\d\+\)\?\>/
syntax match kiroOperator /->\|=>\|??\|?\.\|\.\.\.\|==\|!=\|<=\|>=\|[-=+*\/%<>|]/
syntax match kiroComment /#.*$/
syntax match kiroEscape /\\./ contained

syntax cluster kiroExpression contains=kiroKeyword,kiroConstant,kiroBuiltin,kiroNumber,kiroOperator,kiroSingle,kiroDouble,kiroCommand,kiroInterpolation
syntax region kiroInterpolation matchgroup=kiroInterpolationDelimiter start=+{+ end=+}+ contained oneline contains=`kiroExpression
syntax region kiroSingle start=+'+ skip=+\\.+ end=+'+ oneline contains=kiroEscape
syntax region kiroDouble start=+"+ skip=+\\.+ end=+"+ oneline contains=kiroEscape,kiroInterpolation
syntax region kiroCommand start=+\$\s*"+ skip=+\\.+ end=+"+ oneline contains=kiroEscape,kiroInterpolation
syntax region kiroCommand start=+\%(\$\s*\)\?`+ skip=+\\.+ end=+`+ oneline contains=kiroEscape,kiroInterpolation

highlight default link kiroKeyword Keyword
highlight default link kiroConstant Constant
highlight default link kiroBuiltin Function
highlight default link kiroNumber Number
highlight default link kiroOperator Operator
highlight default link kiroComment Comment
highlight default link kiroSingle String
highlight default link kiroDouble String
highlight default link kiroCommand Special
highlight default link kiroEscape SpecialChar
highlight default link kiroInterpolationDelimiter Special

let b:current_syntax = 'kiro'
