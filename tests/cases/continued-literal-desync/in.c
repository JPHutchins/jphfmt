const char *s = "a\
 b";
#define D(P) \
   if( ((P)->flags&E)!=0 \
       && f(P) ){ goto no_mem;}
