# Route fuel

A vehicle drives along one-way roads between `N` places. Every road burns a fixed amount of fuel and takes a fixed time. Some places have a fuel station that sells fuel at a fixed price per unit. For each of `Q` trips, find the cheapest way to drive from a start place, with a given amount of fuel in the tank, to a target place, and among the cheapest ways the fastest one.

## Rules

- The places are numbered from 1 to `N`. The tank holds at most `C` units of fuel. Fuel comes only in whole units, so the tank always holds an integer amount from 0 to `C`.
- A road goes one way, from a place `U` to a place `V`, needs `A` units of fuel and takes `B` units of time. The vehicle can drive it only while it is at `U` with at least `A` units in the tank. Afterwards the vehicle is at `V` with `A` fewer units. The tank may end up exactly empty. A road with `A = 0` can be driven with an empty tank. A road with `A > C` can never be driven.
- A station at place `V` sells fuel at `P` per unit. Whenever the vehicle is at `V`, it may buy any whole number of units `K >= 0` for `K * P`, as long as the tank then holds at most `C` units. This includes the start of a trip, before the first road, when the start place has a station. Buying takes no time. A place without a station sells no fuel.
- A trip starts at its start place with its given fuel and is a sequence of steps, each of which drives one road or buys fuel. It may drive a road and visit a place, the start and the target included, any number of times. A trip reaches the target when its last step leaves the vehicle at the target. The trip with no steps reaches the target when the start is the target.
- The cost of a trip is the total price it pays for fuel. The fuel in the tank at the start costs nothing, and fuel left in the tank at the end is not refunded. The time of a trip is the sum of the times of the roads it drives, counting a road once for each time it is driven.
- The best trip to the target is one with the lowest cost and, among the trips with that cost, the lowest time. Report its cost and its time. Both numbers are unique even when several trips are best.

## Input

The input is ASCII text read from standard input.

- The first line holds five integers, `N`, `C`, `S`, `M` and `Q`: the number of places, the tank capacity, the number of stations, the number of roads and the number of trips.
- Each of the next `S` lines holds one station as two integers, `V` and `P`: its place and its price per unit.
- Each of the next `M` lines holds one road as four integers, `U`, `V`, `A` and `B`: it goes from `U` to `V`, needs `A` units of fuel and takes `B` units of time.
- Each of the next `Q` lines holds one trip as three integers, `X`, `F` and `Y`: the start place, the fuel in the tank at the start, and the target place.

The integers on a line are separated by one space. Every line, including the last, ends with a line feed (byte 10). Integers are written in decimal with no sign and no leading zeros.

## Output

Write exactly `Q` lines to standard output, one per trip, in input order. When some trip reaches the target, the line holds the cost and the time of the best trip, in decimal, separated by one space. When no trip reaches the target, the line holds the word `unreachable`. Every line ends with a line feed. When `Q` is 0, write nothing.

## Constraints

- `1 <= N <= 50000`
- `1 <= C <= 100`
- `0 <= S <= N`. No two stations are at the same place. `1 <= V <= N` and `0 <= P <= 1000000` for every station.
- `0 <= M <= 300000`. `1 <= U <= N`, `1 <= V <= N`, `0 <= A <= 1000000` and `0 <= B <= 1000000` for every road. A road may start and end at the same place, and several roads may join the same two places in the same direction.
- `0 <= Q <= 100000`. `1 <= X <= N`, `0 <= F <= C` and `1 <= Y <= N` for every trip. Trips may repeat.

Every cost and every time in the output is below 10^13. They can exceed 2^32, which overflows a 32-bit integer.

Some inputs have tens of thousands of places, hundreds of thousands of roads and hundreds of trips, and some of their trips need several stops for fuel. A solution that tries every route is far too slow for them.

## Examples

Input:

```text
4 10 2 5 6
1 5
3 2
1 2 4 10
2 3 3 10
1 3 8 12
3 4 6 5
4 1 0 1
1 0 4
1 10 4
1 7 3
4 0 3
2 0 4
2 0 2
```

Output:

```text
47 25
6 25
0 20
35 21
unreachable
0 0
```

1. From place 1 with an empty tank, the best trip buys 7 units at place 1 for 35, drives through place 2 to place 3, arriving empty, buys 6 units there for 12, and drives to place 4. Its cost is 47 and its time is 10 + 10 + 5 = 25. Driving the direct road from 1 to 3 instead needs 8 units at price 5, so it costs 52.
2. With a full tank, the trip through place 2 arrives at place 3 with 3 units, buys 3 more for 6, and costs 6. The direct road leaves only 2 units at place 3, so it would have to buy 4 units for 8. The cheaper trip wins even though it is slower.
3. With 7 units, the trip through place 2 reaches place 3 with an empty tank and costs nothing. The direct road needs 8 units, so it would cost 5.
4. From place 4, the road to place 1 needs no fuel and takes 1. From there the trip buys 7 units for 35 and drives through place 2 to place 3, as in the first case.
5. Place 2 has no station, and every road out of it needs fuel, so a trip from place 2 with an empty tank reaches nothing.
6. The start is the target, so the trip with no steps is best.

Input:

```text
4 6 2 5 4
1 4
2 1
1 2 2 3
2 1 2 3
1 4 6 1
1 3 3 5
3 4 3 5
1 6 4
1 2 4
1 0 4
3 0 4
```

Output:

```text
0 1
14 7
22 7
unreachable
```

1. With a full tank, both the direct road and the road through place 3 cost nothing, and the direct road is faster.
2. With 2 units, the best trip drives to place 2, buys 6 units there for 6, drives back to place 1 with 4 units, buys 2 more at place 1 for 8, and takes the direct road. Its cost is 14 and its time is 3 + 3 + 1 = 7. Buying 4 units at place 1 right away would cost 16. Going through place 3 also costs 14, but its time is 16.
3. With an empty tank, the trip first buys 2 units at place 1 for 8 to reach place 2, and then goes on as before.
4. Place 3 has no station, and its road needs 3 units.
