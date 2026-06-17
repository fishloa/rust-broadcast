## Table 4 — Application profile encoding
_§5.2.6.1, PDF pp. 21-21_

|  | No. of bits | Identifier | Value |
|---|---|---|---|
| application_profiles_length | 8 | uimsbf |  |
| for( i=0; i<N; i++ ) { |  |  |  |
| application_profile | 16 | uimsbf |  |
| version.major | 8 | uimsbf |  |
| version.minor | 8 | uimsbf |  |
| version.micro | 8 | uimsbf |  |
| } |  |  |  |

