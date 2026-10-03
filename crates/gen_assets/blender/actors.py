"""Two recognizable, non-gory humanoid variants using the approved anatomy/rig."""
from functools import partial


def zombie(humanoid, variant):
    m = humanoid('actor.zombie.'+str(variant), skin='sage_light' if variant == 0 else 'stone',
                 coat='clay' if variant == 0 else 'lavender',
                 trousers='dark_wood', scarf='linen' if variant == 0 else 'sage')
    if variant == 0:
        # Ragged work jacket, patch pocket and crossed repair stitches.
        m.box('jacket.patch', (-.07,-.063,.62), (.065,.008,.07), 'linen')
        m.beam('jacket.stitch.a', (-.095,-.069,.60), (-.048,-.069,.644), .004,.004,'wood')
        m.beam('jacket.stitch.b', (-.095,-.069,.644), (-.048,-.069,.60), .004,.004,'wood')
        for x,z in [(-.083,.44),(-.028,.455),(.04,.448),(.095,.463)]:
            m.mesh('jacket.hem.'+str(x), [(x-.021,-.064,z+.025),(x+.023,-.064,z+.025),(x+.012,-.064,z-.022)], [(0,1,2)], 'clay')
    else:
        # Heavier coat with a broad collar and contrasting diagonal strap.
        for side in [-1,1]:
            m.beam('coat.lapel.'+str(side), (side*.059,-.069,.75), (side*.026,-.069,.64), .036,.013,'linen')
        m.beam('coat.strap', (-.097,-.073,.73), (.087,-.073,.52), .025,.012,'dark_wood')
        for z in [.56,.61,.66]:
            m.box('coat.button.'+str(z), (.012,-.072,z), (.01,.008,.01), 'wood')
    return m


def recipes(humanoid):
    return [partial(zombie, humanoid, variant) for variant in [0,1]]
