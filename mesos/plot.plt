set terminal wxt size 1000,800
set xlabel "Fuzz cases"
set ylabel "Coverage"
set logscale x
set samples 1000000
set key bottom

plot "fuzz_stats.txt" u 2:3 w l,\
	"fuzz_stats_nomutate.txt" u 2:3 w l,\
	"fuzz_stats_first_mutate.txt" u 2:3 w l,\
	"fuzz_stats_nomutate2.txt" u 2:3 w l

